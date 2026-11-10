//! The system's services: the Share menu, printing (and saving as PDF),
//! notifications, and files (Reveal in Finder, Move to Trash, their dates).

use ferrokit_core::kept::{UiCell, UiLocal};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSButton, NSImage, NSPrintInfo, NSPrintOperation, NSPrintingPaginationMode, NSSharingServicePicker, NSView, NSWindow};
use objc2_foundation::{NSArray, NSBundle, NSError, NSOperationQueue, NSRectEdge, NSSize, NSString, NSURL};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse,
    UNNotificationSound, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};

use crate::appkit_views::ActionTarget;
use crate::view::{AnyView, Cx, Node, Size, View};

// ---- sharing ----

/// Something to share (see [`share_button`]).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ShareItem {
    /// Plain text.
    Text(String),
    /// A web address.
    Url(String),
    /// A file, shared as itself (AirDrop sends it, Mail attaches it).
    File(PathBuf),
}

pub(crate) fn share_objects(items: &[ShareItem]) -> Retained<NSArray<AnyObject>> {
    let objects: Vec<Retained<AnyObject>> = items
        .iter()
        .filter_map(|i| match i {
            ShareItem::Text(t) => Some(Retained::into_super(Retained::into_super(NSString::from_str(t)))),
            ShareItem::Url(u) => NSURL::URLWithString(&NSString::from_str(u)).map(|u| Retained::into_super(Retained::into_super(u))),
            ShareItem::File(p) => Some(Retained::into_super(Retained::into_super(NSURL::fileURLWithPath(&NSString::from_str(&p.to_string_lossy()))))),
        })
        .collect();
    NSArray::from_retained_slice(&objects)
}

/// A Share button (see [`share_button`]).
pub struct ShareButton {
    items: Items,
    title: Option<String>,
}

impl ShareButton {
    /// A titled button ("Share…") instead of the icon alone.
    pub fn title(mut self, title: &str) -> Self {
        self.title = Some(title.into());
        self
    }
}

type Items = Rc<dyn Fn() -> Vec<ShareItem>>;

thread_local! {
    /// What each share button shares, for checks.
    static SHARES: UiCell<Vec<(objc2::rc::Weak<NSButton>, Items)>> = const { UiCell::new(Vec::new()) };
}

/// What the share button `button` would share now.
pub(crate) fn shared_by(button: &NSButton) -> Option<Vec<ShareItem>> {
    let f = SHARES.with_borrow(|s| s.iter().find(|(b, _)| b.load().is_some_and(|b| std::ptr::eq(&*b, button))).map(|(_, f)| f.clone()))?;
    Some(f())
}

impl View for ShareButton {
    fn build(self, cx: &mut Cx) -> Node {
        let mtm = cx.mtm;
        let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str("square.and.arrow.up"), Some(&NSString::from_str("Share")));
        let button = match (&self.title, image) {
            (None, Some(img)) => {
                // SAFETY: a plain image button; the target is set below.
                unsafe { NSButton::buttonWithImage_target_action(&img, None, None, mtm) }
            }
            (title, _) => {
                // SAFETY: as above.
                unsafe { NSButton::buttonWithTitle_target_action(&NSString::from_str(title.as_deref().unwrap_or("Share…")), None, None, mtm) }
            }
        };
        let picker: Rc<RefCell<Option<Retained<NSSharingServicePicker>>>> = Rc::default();
        let (items, b, held) = (self.items.clone(), objc2::rc::Weak::from_retained(&button), picker.clone());
        let target = ActionTarget::new(mtm, move || {
            let Some(b) = b.load() else { return };
            // SAFETY: an array of strings and URLs, which the picker takes.
            let p = unsafe { NSSharingServicePicker::initWithItems(NSSharingServicePicker::alloc(), &share_objects(&items())) };
            p.showRelativeToRect_ofView_preferredEdge(b.bounds(), &b, NSRectEdge::MinY);
            *held.borrow_mut() = Some(p);
        });
        // SAFETY: the target answers fire:; the node keeps it.
        unsafe {
            button.setTarget(Some(&target));
            button.setAction(Some(sel!(fire:)));
        }
        SHARES.with_borrow_mut(|s| {
            s.retain(|(b, _)| b.load().is_some());
            s.push((objc2::rc::Weak::from_retained(&button), self.items));
        });
        let (mut node, _) = Node::leaf(cx, Retained::into_super(Retained::into_super(button)), (false, false), Size::default());
        node.keep(Retained::into_super(target));
        node.hold(picker);
        node
    }
}

/// A button showing the system's Share menu (Mail, Messages, AirDrop,
/// Notes…) for what `items()` returns when it's clicked.
pub fn share_button(items: impl Fn() -> Vec<ShareItem> + 'static) -> ShareButton {
    ShareButton { items: Rc::new(items), title: None }
}

// ---- printing ----

/// Lays `view` out at the printable width of `info`'s page.
fn printable(view: AnyView, info: &NSPrintInfo) -> Retained<NSView> {
    let paper = info.paperSize();
    let width = (paper.width - info.leftMargin() - info.rightMargin()).max(100.0);
    let (root, size) = crate::host_view(view);
    // Paper is white: printed in the light appearance, whatever the screen's.
    if let Some(light) = objc2_app_kit::NSAppearance::appearanceNamed(unsafe { objc2_app_kit::NSAppearanceNameAqua }) {
        objc2_app_kit::NSAppearanceCustomization::setAppearance(&*root, Some(&light));
    }
    // Inside a batch (a button's action), the view's effects (labels'
    // text, charts' drawing) haven't run yet: run them, measure again, lay
    // out at the page's width, and run what that changed (charts' sizes).
    crate::signal::run_pending();
    let height = crate::hosted_ideal(&root).map_or(size.height, |s| s.height);
    root.setFrameSize(NSSize::new(width, height));
    crate::signal::run_pending();
    root
}

fn print_info() -> Retained<NSPrintInfo> {
    let info = NSPrintInfo::sharedPrintInfo();
    // SAFETY: NSPrintInfo copies as NSPrintInfo.
    let info: Retained<NSPrintInfo> = unsafe { msg_send![&*info, copy] };
    // Half-inch margins where the printer gives none (no printer set up).
    if info.leftMargin() < 18.0 {
        info.setLeftMargin(36.0);
        info.setRightMargin(36.0);
    }
    if info.topMargin() < 18.0 {
        info.setTopMargin(36.0);
        info.setBottomMargin(36.0);
    }
    info.setHorizontalPagination(NSPrintingPaginationMode::Fit);
    info.setVerticalPagination(NSPrintingPaginationMode::Automatic);
    info.setHorizontallyCentered(false);
    info.setVerticallyCentered(false);
    info
}

define_class!(
    // Frees a printed view's tree when its print sheet ends.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FerrokitPrintDone"]
    #[ivars = RefCell<Option<Retained<NSView>>>]
    struct PrintDone;

    impl PrintDone {
        #[unsafe(method(printOperationDidRun:success:contextInfo:))]
        fn done(&self, _op: &NSPrintOperation, _ok: Bool, _info: *mut std::ffi::c_void) {
            if let Some(root) = self.ivars().borrow_mut().take() {
                crate::drop_hosted(&root);
            }
            PRINTING.with_borrow_mut(|p| p.retain(|d| !std::ptr::eq(&**d, self)));
        }
    }
);

thread_local! {
    static PRINTING: UiCell<Vec<Retained<PrintDone>>> = const { UiCell::new(Vec::new()) };
}

/// Prints `view` laid out at the page's width, across as many pages as it
/// needs, with the Print sheet on `window` (printer, copies, Save as PDF).
///
/// The view is built for printing on its own: text, pictures, symbols,
/// charts, backgrounds and borders print; shadows and materials don't.
pub fn print(window: &NSWindow, title: &str, view: impl View) {
    let info = print_info();
    let root = printable(AnyView::new(view), &info);
    let op = NSPrintOperation::printOperationWithView_printInfo(&root, &info);
    op.setJobTitle(Some(&NSString::from_str(title)));
    let done = PrintDone::alloc(crate::mtm()).set_ivars(RefCell::new(Some(root)));
    // SAFETY: NSObject's designated initialiser.
    let done: Retained<PrintDone> = unsafe { msg_send![super(done), init] };
    PRINTING.with_borrow_mut(|p| p.push(done.clone()));
    // SAFETY: the delegate answers the selector and is kept until it runs.
    unsafe { op.runOperationModalForWindow_delegate_didRunSelector_contextInfo(window, Some(&done), Some(sel!(printOperationDidRun:success:contextInfo:)), std::ptr::null_mut()) };
}

/// Saves `view` as a PDF at `path`, on letter or A4 pages (the user's
/// default paper) as [`print()`] would print it, with no panel.
pub fn save_pdf(view: impl View, path: impl AsRef<Path>) -> std::io::Result<()> {
    let path = std::path::absolute(path)?;
    let info = print_info();
    // Printing to a file paginates; AppKit's PDF operation makes one page.
    // SAFETY: AppKit's job keys, with a string and a file URL.
    unsafe {
        info.setJobDisposition(objc2_app_kit::NSPrintSaveJob);
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        info.dictionary().setObject_forKey(&*url, objc2::runtime::ProtocolObject::from_ref(objc2_app_kit::NSPrintJobSavingURL));
    }
    let root = printable(AnyView::new(view), &info);
    let op = NSPrintOperation::printOperationWithView_printInfo(&root, &info);
    op.setShowsPrintPanel(false);
    op.setShowsProgressPanel(false);
    let ok = op.runOperation();
    crate::drop_hosted(&root);
    if ok && path.exists() { Ok(()) } else { Err(std::io::Error::other("printing to PDF failed")) }
}

/// `view` as a PDF (see [`save_pdf`]).
pub fn pdf_data(view: impl View) -> Vec<u8> {
    thread_local!(static N: Cell<u32> = const { Cell::new(0) });
    let n = N.with(|c| c.replace(c.get() + 1));
    let path = std::env::temp_dir().join(format!("ferrokit-print-{}-{n}.pdf", std::process::id()));
    let data = save_pdf(view, &path).and_then(|()| std::fs::read(&path)).unwrap_or_default();
    let _ = std::fs::remove_file(&path);
    data
}

// ---- notifications ----

/// A notification (see [`notify`]).
pub struct Notification {
    id: String,
    title: String,
    subtitle: String,
    body: String,
    sound: bool,
    daily: Option<(u32, u32)>,
    on_click: Option<Rc<dyn Fn()>>,
}

impl Notification {
    /// A notification titled `title`.
    pub fn new(title: &str) -> Self {
        thread_local!(static NEXT: Cell<u64> = const { Cell::new(1) });
        let n = NEXT.with(|c| c.replace(c.get() + 1));
        Notification { id: format!("ferrokit-{n}"), title: title.into(), subtitle: String::new(), body: String::new(), sound: false, daily: None, on_click: None }
    }

    /// A second line under the title, in bold.
    pub fn subtitle(mut self, text: &str) -> Self {
        self.subtitle = text.into();
        self
    }

    /// The message.
    pub fn body(mut self, text: &str) -> Self {
        self.body = text.into();
        self
    }

    /// Plays the system's notification sound.
    pub fn sound(mut self) -> Self {
        self.sound = true;
        self
    }

    /// Its identifier: a new notification with the same one replaces it in
    /// Notification Center (a download's progress, today's summary).
    pub fn id(mut self, id: &str) -> Self {
        self.id = id.into();
        self
    }

    /// Shows it every day at `hour:minute` local time (a habit's reminder)
    /// instead of now, until [`cancel_notification`] with its
    /// [`id`](Self::id). Posting another with the same id replaces it.
    pub fn daily_at(mut self, hour: u32, minute: u32) -> Self {
        self.daily = Some((hour.min(23), minute.min(59)));
        self
    }

    /// Runs `action` when the user clicks the notification (the app comes
    /// to the front first), while the app is still running.
    pub fn on_click(mut self, action: impl Fn() + 'static) -> Self {
        self.on_click = Some(Rc::new(crate::signal::with_current_owner(action)));
        self
    }
}

/// A sent notification's text, as [`testing::sent_notifications`](crate::testing::sent_notifications) reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentNotification {
    /// Its identifier.
    pub id: String,
    /// Its title.
    pub title: String,
    /// Its subtitle.
    pub subtitle: String,
    /// Its message.
    pub body: String,
    /// Whether it plays a sound.
    pub sound: bool,
    /// The time it repeats at every day, if it does.
    pub daily: Option<(u32, u32)>,
}

thread_local! {
    static CLICKS: UiCell<HashMap<String, Rc<dyn Fn()>>> = UiCell::new(HashMap::new());
    static DELEGATE: UiCell<Option<Retained<NotificationDelegate>>> = const { UiCell::new(None) };
    pub(crate) static HOLD: Cell<bool> = const { Cell::new(false) };
    pub(crate) static SENT: UiCell<Vec<SentNotification>> = const { UiCell::new(Vec::new()) };
}

define_class!(
    // Shows notifications while the app is in front too, and routes clicks
    // to their actions. UserNotifications may call it on any thread.
    #[unsafe(super(NSObject))]
    #[name = "FerrokitNotifications"]
    struct NotificationDelegate;

    unsafe impl NSObjectProtocol for NotificationDelegate {}

    unsafe impl UNUserNotificationCenterDelegate for NotificationDelegate {
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(&self, _c: &UNUserNotificationCenter, _n: &UNNotification, done: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>) {
            done.call((UNNotificationPresentationOptions::Banner | UNNotificationPresentationOptions::List | UNNotificationPresentationOptions::Sound,));
        }

        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn received(&self, _c: &UNUserNotificationCenter, response: &UNNotificationResponse, done: &block2::DynBlock<dyn Fn()>) {
            let id = response.notification().request().identifier().to_string();
            let block = RcBlock::new(move || {
                crate::failure::guard("a notification's click", false, || click(&id));
            });
            // SAFETY: the block runs on the main queue, where the actions live.
            unsafe { NSOperationQueue::mainQueue().addOperationWithBlock(&block) };
            done.call(());
        }
    }
);

/// Runs the click action of the notification `id` (on the main thread).
pub(crate) fn click(id: &str) -> bool {
    let action = CLICKS.with_borrow(|c| c.get(id).cloned());
    if let Some(a) = &action {
        crate::signal::batch(|| a());
    }
    action.is_some()
}

/// Whether this app can post notifications: it must run from an app
/// bundle with a bundle identifier (a plain `cargo run` binary can't).
pub fn notifications_supported() -> bool {
    NSBundle::mainBundle().bundleIdentifier().is_some()
}

/// Posts `notification` through Notification Center. The first time, macOS
/// asks the user whether to allow the app's notifications; while the app is
/// in front they show as banners too. Without an app bundle
/// ([`notifications_supported`]) it does nothing.
pub fn notify(notification: Notification) {
    if let Some(a) = &notification.on_click {
        CLICKS.with_borrow_mut(|c| c.insert(notification.id.clone(), a.clone()));
    }
    if HOLD.with(Cell::get) {
        let n = &notification;
        SENT.with_borrow_mut(|s| s.retain(|old| old.id != n.id));
        SENT.with_borrow_mut(|s| s.push(SentNotification { id: n.id.clone(), title: n.title.clone(), subtitle: n.subtitle.clone(), body: n.body.clone(), sound: n.sound, daily: n.daily }));
        return;
    }
    if !notifications_supported() {
        return;
    }
    listen_for_notifications();
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(&notification.title));
    content.setSubtitle(&NSString::from_str(&notification.subtitle));
    content.setBody(&NSString::from_str(&notification.body));
    if notification.sound {
        content.setSound(Some(&UNNotificationSound::defaultSound()));
    }
    let trigger = notification.daily.map(|(hour, minute)| {
        let when = objc2_foundation::NSDateComponents::new();
        when.setHour(hour as isize);
        when.setMinute(minute as isize);
        Retained::into_super(objc2_user_notifications::UNCalendarNotificationTrigger::triggerWithDateMatchingComponents_repeats(&when, true))
    });
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(&notification.id), &content, trigger.as_deref());
    // Asking again is free once the user has answered; the request goes
    // out when they have.
    let block = RcBlock::new(move |granted: Bool, _e: *mut NSError| {
        if granted.as_bool() {
            UNUserNotificationCenter::currentNotificationCenter().addNotificationRequest_withCompletionHandler(&request, None);
        }
    });
    center.requestAuthorizationWithOptions_completionHandler(UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound, &block);
}

/// Removes the notification `id`: a daily one stops repeating, and one
/// already shown leaves Notification Center.
pub fn cancel_notification(id: &str) {
    CLICKS.with_borrow_mut(|c| c.remove(id));
    if HOLD.with(Cell::get) {
        SENT.with_borrow_mut(|s| s.retain(|n| n.id != id));
        return;
    }
    if !notifications_supported() {
        return;
    }
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let ids = NSArray::from_retained_slice(&[NSString::from_str(id)]);
    center.removePendingNotificationRequestsWithIdentifiers(&ids);
    center.removeDeliveredNotificationsWithIdentifiers(&ids);
}

// ---- files ----

thread_local! {
    /// What `reveal_in_finder` was asked to show, while checks capture it
    /// (`testing::capture_reveals`) instead of opening Finder windows.
    static REVEALS: UiCell<Option<Vec<Vec<PathBuf>>>> = const { UiCell::new(None) };
}

pub(crate) fn capture_reveals(on: bool) {
    REVEALS.with_borrow_mut(|r| *r = on.then(Vec::new));
}

pub(crate) fn take_reveals() -> Vec<Vec<PathBuf>> {
    REVEALS.with_borrow_mut(|r| r.as_mut().map(std::mem::take).unwrap_or_default())
}

/// Shows `paths` selected in the Finder (File ▸ Show in Finder, ⌘R in
/// many apps): one Finder window per folder, the items selected, the
/// Finder brought to the front.
pub fn reveal_in_finder<P: AsRef<Path>>(paths: &[P]) {
    let paths: Vec<PathBuf> = paths.iter().map(|p| p.as_ref().to_path_buf()).collect();
    if REVEALS.with_borrow_mut(|r| r.as_mut().map(|r| r.push(paths.clone())).is_some()) {
        return;
    }
    let urls: Vec<Retained<NSURL>> = paths.iter().map(|p| NSURL::fileURLWithPath(&NSString::from_str(&p.to_string_lossy()))).collect();
    objc2_app_kit::NSWorkspace::sharedWorkspace().activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&urls));
}

/// Moves `path` (a file or a folder) to the Trash of its volume, as the
/// Finder's Move to Trash does, and returns where it is now: keep it to
/// put the item back (undo) with [`std::fs::rename`] to the old path. The
/// name in the Trash can differ from the old one when the Trash already
/// has an item with that name.
pub fn move_to_trash(path: impl AsRef<Path>) -> std::io::Result<PathBuf> {
    let path = path.as_ref();
    if std::fs::symlink_metadata(path).is_err() {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, format!("{} doesn't exist", path.display())));
    }
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    let mut result: Option<Retained<NSURL>> = None;
    objc2_foundation::NSFileManager::defaultManager()
        .trashItemAtURL_resultingItemURL_error(&url, Some(&mut result))
        .map_err(|e| std::io::Error::other(e.localizedDescription().to_string()))?;
    result.and_then(|u| u.path()).map(|p| PathBuf::from(p.to_string())).ok_or_else(|| std::io::Error::other("the Trash didn't say where the item went"))
}

/// Opens `url` in the app that handles it: a web page in the browser, a
/// `mailto:` in Mail, a System Settings pane (such as
/// [`FULL_DISK_ACCESS_SETTINGS`]). False if no app handles it.
pub fn open_url(url: &str) -> bool {
    match NSURL::URLWithString(&NSString::from_str(url)) {
        Some(url) => objc2_app_kit::NSWorkspace::sharedWorkspace().openURL(&url),
        None => false,
    }
}

/// System Settings ▸ Privacy & Security ▸ Full Disk Access, for
/// [`open_url`]: where the user allows an app to read every folder.
pub const FULL_DISK_ACCESS_SETTINGS: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles";

/// Whether this app has Full Disk Access. It never prompts (it tries to
/// read the privacy database, which only apps with access can). An app
/// gets access after a relaunch, so read it at launch or before a walk.
///
/// In the App Sandbox this is false whatever the user allowed, unless the
/// app holds a folder the user chose that contains the home folder's
/// `Library`: the sandbox keeps the app to its container and the folders
/// the user chose, with or without Full Disk Access, and hides the
/// privacy database with the rest. A sandboxed app has no use for the
/// answer outside those folders, so don't send its users to System
/// Settings on a false here.
pub fn has_full_disk_access() -> bool {
    real_home().is_some_and(|h| std::fs::File::open(h.join("Library/Application Support/com.apple.TCC/TCC.db")).is_ok())
}

/// Whether reading the folder at `path` may stop to ask the user's
/// consent, so an app walking the disk should stay out of it (show it as
/// "not scanned" instead). `full_disk_access` is what
/// [`has_full_disk_access`] says; read it once per walk.
///
/// A file call that waits for consent can't be cancelled, and many at once
/// (a parallel walk) tie up the system's sandbox checks until the app
/// quits, which can make the whole Mac stall. Without Full Disk Access
/// these ask: Desktop, Documents, Downloads, the Trash and `~/Library` in
/// the home folder (other apps' data, iCloud Drive, cloud storage), and
/// other volumes (`/Volumes/…`: removable and network ones). The media
/// libraries ask for their own consent even with it: `~/Music/Music`,
/// `~/Music/iTunes`, `~/Music/GarageBand`, `~/Movies/TV`, `~/Pictures/Photo
/// Booth Library`, and packages ending in `.photoslibrary`,
/// `.photolibrary`, `.musiclibrary` or `.tvlibrary`.
///
/// The path is read as text, with the startup disk's other names for the
/// same folders taken into account (`/System/Volumes/Data/…`, its entry in
/// `/Volumes`, another case of the letters) and a symbolic link followed
/// one step; nothing here opens the folder.
///
/// A folder the user chose in an Open panel can be read whatever this
/// says: choosing it gave consent.
pub fn asks_before_reading(path: impl AsRef<Path>, full_disk_access: bool) -> bool {
    real_home().is_some_and(|home| asks(&spelled_plainly(path.as_ref()), &home, full_disk_access))
}

/// `path` as the folders that ask are written: absolute, a symbolic link
/// followed (a link into Documents asks as Documents does), and without
/// the startup disk's other names for the same place
/// (`/System/Volumes/Data/Users/…`, `/Volumes/Macintosh HD/Users/…`).
fn spelled_plainly(path: &Path) -> PathBuf {
    let mut path = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(path) };
    // Reading a link's target doesn't read the folder it points at.
    if let Ok(target) = std::fs::read_link(&path) {
        path = path.parent().unwrap_or(Path::new("/")).join(target);
    }
    let path = without_dots(&path);
    let path = without_data_volume(&path);
    if path.starts_with("/Volumes") && path.components().count() > 2 && is_boot_volume(&path) {
        return Path::new("/").join(path.components().skip(3).collect::<PathBuf>());
    }
    path
}

/// `path` with `.` and `..` worked out (in the text only).
fn without_dots(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

/// `/System/Volumes/Data/x` is `/x`: the data volume, where the home
/// folders are, under the name a walk from `/` meets it by.
fn without_data_volume(path: &Path) -> PathBuf {
    match path.strip_prefix("/System/Volumes/Data") {
        Ok(rest) => Path::new("/").join(rest),
        Err(_) => path.to_path_buf(),
    }
}

/// Whether `path` is `folder` or inside it, whatever the letters' case (the
/// startup disk doesn't tell `documents` from `Documents`).
fn inside(path: &Path, folder: &Path) -> bool {
    let (mut p, mut f) = (path.components(), folder.components());
    loop {
        match (p.next(), f.next()) {
            (_, None) => return true,
            (Some(a), Some(b)) if a.as_os_str().eq_ignore_ascii_case(b.as_os_str()) => {}
            _ => return false,
        }
    }
}

fn asks(path: &Path, home: &Path, full_disk_access: bool) -> bool {
    const MEDIA: [&str; 5] = ["Music/Music", "Music/iTunes", "Music/GarageBand", "Movies/TV", "Pictures/Photo Booth Library"];
    const LIBRARIES: [&str; 4] = ["photoslibrary", "photolibrary", "musiclibrary", "tvlibrary"];
    if MEDIA.iter().any(|m| inside(path, &home.join(m))) || path.components().any(|c| Path::new(c.as_os_str()).extension().is_some_and(|e| LIBRARIES.iter().any(|l| e.eq_ignore_ascii_case(l)))) {
        return true;
    }
    if full_disk_access {
        return false;
    }
    const PROTECTED: [&str; 5] = ["Desktop", "Documents", "Downloads", ".Trash", "Library"];
    PROTECTED.iter().any(|p| inside(path, &home.join(p))) || (path.starts_with("/Volumes") && path.components().count() > 2 && !is_boot_volume(path))
}

/// Whether `path` is under the startup disk's own entry in `/Volumes` (a
/// link to `/`).
fn is_boot_volume(path: &Path) -> bool {
    let volume: PathBuf = path.components().take(3).collect();
    std::fs::canonicalize(&volume).is_ok_and(|v| v == Path::new("/"))
}

/// The user's real home folder, also in the App Sandbox (where `HOME` is
/// the app's container).
fn real_home() -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    // SAFETY: getpwuid_r fills `pw` with pointers into `buf`, which outlives their use.
    unsafe {
        let mut pw: libc::passwd = std::mem::zeroed();
        let mut buf = vec![0 as libc::c_char; 4096];
        let mut out: *mut libc::passwd = std::ptr::null_mut();
        if libc::getpwuid_r(libc::getuid(), &mut pw, buf.as_mut_ptr(), buf.len(), &mut out) != 0 || out.is_null() || pw.pw_dir.is_null() {
            return None;
        }
        Some(PathBuf::from(std::ffi::OsStr::from_bytes(std::ffi::CStr::from_ptr(pw.pw_dir).to_bytes())))
    }
}

/// Makes the app the one notifications are shown through and clicked in:
/// at launch (a reminder scheduled in an earlier run arrives while the app
/// is in front, or is clicked), and before the first [`notify`].
pub(crate) fn listen_for_notifications() {
    if !notifications_supported() {
        return;
    }
    DELEGATE.with_borrow_mut(|d| {
        if d.is_none() {
            // SAFETY: NSObject's designated initialiser.
            let delegate: Retained<NotificationDelegate> = unsafe { msg_send![NotificationDelegate::alloc(), init] };
            UNUserNotificationCenter::currentNotificationCenter().setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            *d = Some(delegate);
        }
    });
}

/// A file's dates (see [`file_dates`]); `None` where the file system
/// doesn't keep one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct FileDates {
    /// When its content last changed.
    pub modified: Option<std::time::SystemTime>,
    /// When it was made.
    pub created: Option<std::time::SystemTime>,
    /// When it was put in its folder (moved, copied or downloaded there).
    /// Copying files to a new Mac, or restoring them, resets it.
    pub added: Option<std::time::SystemTime>,
    /// When it was last opened in an app (the date Spotlight calls Last
    /// Opened, `kMDItemLastUsedDate`); `None` if never.
    pub last_opened: Option<std::time::SystemTime>,
}

/// The dates of the file or folder at `path` (not following a symbolic
/// link): for "last opened" lists, "date added" columns and the like.
/// Reading them is cheap (no Spotlight query): a few thousand files take a
/// few milliseconds.
pub fn file_dates(path: impl AsRef<Path>) -> std::io::Result<FileDates> {
    use std::os::unix::ffi::OsStrExt;
    let path = path.as_ref();
    let meta = std::fs::symlink_metadata(path)?;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(std::io::Error::other)?;
    let at = |secs: i64| (secs > 0).then(|| std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs as u64));
    Ok(FileDates { modified: meta.modified().ok(), created: meta.created().ok(), added: date_added(&c).and_then(at), last_opened: last_opened(&c).and_then(at) })
}

/// The extended attribute macOS keeps the last-opened date in (a
/// `timespec`, seconds first, little-endian).
const LAST_USED: &std::ffi::CStr = c"com.apple.lastuseddate#PS";

fn last_opened(path: &std::ffi::CStr) -> Option<i64> {
    let mut value = [0u8; 16];
    // SAFETY: NUL-terminated strings; `value` has room for the length given.
    let n = unsafe { libc::getxattr(path.as_ptr(), LAST_USED.as_ptr(), value.as_mut_ptr().cast(), value.len(), 0, libc::XATTR_NOFOLLOW) };
    (n >= 8).then(|| i64::from_le_bytes(value[..8].try_into().expect("8 bytes")))
}

fn date_added(path: &std::ffi::CStr) -> Option<i64> {
    // The reply: its length, then the attribute.
    #[repr(C, packed(4))]
    struct Reply {
        length: u32,
        added: libc::timespec,
    }
    // SAFETY: plain C structs, valid zeroed.
    let mut list: libc::attrlist = unsafe { std::mem::zeroed() };
    list.bitmapcount = libc::ATTR_BIT_MAP_COUNT;
    list.commonattr = libc::ATTR_CMN_ADDEDTIME;
    let mut reply: Reply = unsafe { std::mem::zeroed() };
    // SAFETY: `list` asks for one timespec, which `reply` has room for.
    let r = unsafe { libc::getattrlist(path.as_ptr(), (&raw mut list).cast(), (&raw mut reply).cast(), size_of::<Reply>(), libc::FSOPT_NOFOLLOW) };
    let (length, secs) = (reply.length, reply.added.tv_sec);
    (r == 0 && length as usize >= size_of::<Reply>()).then_some(secs)
}

/// Sets a file's last-opened date, for tests (`None` removes it).
pub(crate) fn set_last_opened(path: &Path, secs: Option<i64>) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(std::io::Error::other)?;
    // SAFETY: NUL-terminated strings; a buffer of the length given.
    let r = match secs {
        Some(s) => {
            let mut value = [0u8; 16];
            value[..8].copy_from_slice(&s.to_le_bytes());
            unsafe { libc::setxattr(c.as_ptr(), LAST_USED.as_ptr(), value.as_ptr().cast(), value.len(), 0, libc::XATTR_NOFOLLOW) }
        }
        None => unsafe { libc::removexattr(c.as_ptr(), LAST_USED.as_ptr(), libc::XATTR_NOFOLLOW) },
    };
    if r == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

#[cfg(test)]
mod file_tests {
    use super::*;

    #[test]
    fn dates_of_a_file() {
        let dir = std::env::temp_dir().join(format!("ferrokit-dates-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, "a").unwrap();
        let fresh = file_dates(&file).unwrap();
        let now = std::time::SystemTime::now();
        let recent = |t: Option<std::time::SystemTime>| t.is_some_and(|t| now.duration_since(t).map_or(true, |d| d.as_secs() < 60));
        assert!(recent(fresh.modified) && recent(fresh.created) && recent(fresh.added), "{fresh:?}");
        assert_eq!(fresh.last_opened, None);
        set_last_opened(&file, Some(1_600_000_000)).unwrap();
        assert_eq!(file_dates(&file).unwrap().last_opened, Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000)));
        assert!(file_dates(dir.join("missing")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reveals_are_captured() {
        capture_reveals(true);
        reveal_in_finder(&["/tmp/a", "/tmp/b"]);
        assert_eq!(take_reveals(), vec![vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")]]);
        capture_reveals(false);
    }

    #[test]
    fn trashing_a_missing_file_fails() {
        assert_eq!(move_to_trash("/nonexistent/ferrokit-nothing").unwrap_err().kind(), std::io::ErrorKind::NotFound);
    }
}

#[cfg(test)]
mod access_tests {
    use super::*;

    #[test]
    fn folders_that_ask() {
        let home = Path::new("/Users/someone");
        let asks = |p: &str, fda: bool| asks(Path::new(p), home, fda);
        // Without Full Disk Access.
        for p in ["/Users/someone/Documents", "/Users/someone/Desktop/a", "/Users/someone/Downloads", "/Users/someone/.Trash", "/Users/someone/Library/Containers/x", "/Volumes/Backup/photos"] {
            assert!(asks(p, false), "{p} asks without access");
            assert!(!asks(p, true), "{p} is readable with access");
        }
        for p in ["/Users/someone", "/Users/someone/Projects/app", "/Users/someone/DocumentsOld", "/Applications", "/Library/Fonts", "/Volumes"] {
            assert!(!asks(p, false), "{p} doesn't ask");
        }
        // The media libraries ask even with access.
        for p in ["/Users/someone/Music/Music", "/Users/someone/Music/GarageBand/x", "/Users/someone/Movies/TV", "/Users/someone/Pictures/Photo Booth Library", "/Users/someone/Pictures/Photos Library.photoslibrary", "/Users/someone/Projects/Old.PhotosLibrary/masters"] {
            assert!(asks(p, true), "{p} asks even with access");
        }
        assert!(!asks("/Users/someone/Music/Loops", true));
        // Other spellings of the same folders.
        assert!(asks("/users/someone/documents/x", false), "the startup disk ignores case");
        assert_eq!(without_data_volume(Path::new("/System/Volumes/Data/Users/someone/Documents")), Path::new("/Users/someone/Documents"));
        assert_eq!(without_data_volume(Path::new("/System/Volumes/Data")), Path::new("/"));
        assert_eq!(without_dots(Path::new("/Users/someone/Projects/../Documents/./a")), Path::new("/Users/someone/Documents/a"));
        let boot = std::fs::read_dir("/Volumes").ok().and_then(|d| d.flatten().map(|e| e.path()).find(|p| is_boot_volume(p)));
        if let (Some(boot), Some(home)) = (boot, real_home()) {
            let other = boot.join(home.strip_prefix("/").unwrap()).join("Documents");
            assert_eq!(spelled_plainly(&other), home.join("Documents"), "the startup disk under /Volumes");
            assert!(asks_before_reading(&other, false));
            assert!(asks_before_reading(Path::new("/System/Volumes/Data").join(home.strip_prefix("/").unwrap()).join("Desktop"), false));
        }
        // A link into a folder that asks, asks.
        let link = std::env::temp_dir().join(format!("ferrokit-link-{}", std::process::id()));
        if let Some(home) = real_home()
            && std::os::unix::fs::symlink(home.join("Downloads"), &link).is_ok()
        {
            assert!(asks_before_reading(&link, false), "a link to Downloads");
            let _ = std::fs::remove_file(&link);
        }
        // The real home and the access check run without prompting.
        assert!(real_home().is_some_and(|h| h.is_absolute()));
        let _ = has_full_disk_access();
    }
}
