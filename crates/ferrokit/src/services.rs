//! The system's services: the Share menu, printing (and saving as PDF), and
//! notifications.

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
    static SHARES: RefCell<Vec<(objc2::rc::Weak<NSButton>, Items)>> = const { RefCell::new(Vec::new()) };
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
    static PRINTING: RefCell<Vec<Retained<PrintDone>>> = const { RefCell::new(Vec::new()) };
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
    static CLICKS: RefCell<HashMap<String, Rc<dyn Fn()>>> = RefCell::new(HashMap::new());
    static DELEGATE: RefCell<Option<Retained<NotificationDelegate>>> = const { RefCell::new(None) };
    pub(crate) static HOLD: Cell<bool> = const { Cell::new(false) };
    pub(crate) static SENT: RefCell<Vec<SentNotification>> = const { RefCell::new(Vec::new()) };
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
                click(&id);
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
    let center = UNUserNotificationCenter::currentNotificationCenter();
    DELEGATE.with_borrow_mut(|d| {
        if d.is_none() {
            // SAFETY: NSObject's designated initialiser.
            let delegate: Retained<NotificationDelegate> = unsafe { msg_send![NotificationDelegate::alloc(), init] };
            center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            *d = Some(delegate);
        }
    });
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
