//! Sheets: alerts, views in a sheet, and the Open and Save panels. Every
//! callback runs in a batch, once.

use std::cell::RefCell;
use std::path::PathBuf;

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSModalResponse, NSModalResponseOK, NSOpenPanel, NSSavePanel,
    NSWindow,
};
use objc2_foundation::{NSArray, NSString, NSURL};

use crate::Window;

fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("main thread")
}

/// Where a dialog's completion finds the sender of its [`Answer`](crate::Answer).
type Pending<T> = std::rc::Rc<RefCell<Option<crate::task::Responder<T>>>>;

/// Wraps a one-shot callback for an AppKit completion block.
fn once<A: 'static>(f: impl FnOnce(A) + 'static) -> impl Fn(A) + 'static {
    let cell = RefCell::new(Some(f));
    move |a| {
        let f = cell.borrow_mut().take();
        if let Some(f) = f {
            crate::failure::entry("a sheet's or panel's completion", (), || f(a));
        }
    }
}

/// An alert shown as a sheet (see [`alert`]).
pub struct Alert {
    title: String,
    message: String,
    buttons: Vec<String>,
    destructive: Option<usize>,
    warning: bool,
}

impl Alert {
    /// An alert with a title and an informative message.
    pub fn new(title: &str, message: &str) -> Self {
        Alert { title: title.into(), message: message.into(), buttons: Vec::new(), destructive: None, warning: false }
    }

    /// Adds a button (the first is the default; Return presses it, Escape
    /// presses one titled "Cancel").
    pub fn button(mut self, title: &str) -> Self {
        self.buttons.push(title.into());
        self
    }

    /// Marks button `index` as destructive (shown in red).
    pub fn destructive(mut self, index: usize) -> Self {
        self.destructive = Some(index);
        self
    }

    /// Shows the warning style (a caution icon).
    pub fn warning(mut self) -> Self {
        self.warning = true;
        self
    }
}

/// Shows `alert` as a sheet on `window`; `on_response` gets the index of the
/// button chosen.
pub fn alert(window: &NSWindow, alert: Alert, on_response: impl FnOnce(usize) + 'static) {
    show_alert(window, alert, move |i| {
        if let Some(i) = i {
            on_response(i);
        }
    });
}

/// Shows `alert` as a sheet on `window`, and returns a future for the index
/// of the button chosen: [`alert`] for a [`spawn_local`](crate::spawn_local)
/// task. `None` when the alert ends without a button (its window closed)
/// and it has no Cancel button to answer as; dropping the future (the task
/// was cancelled) closes the alert.
///
/// ```no_run
/// use ferrokit::*;
///
/// fn delete(window: Retained<NSWindow>, deleted: Signal<bool>) {
///     spawn_local(async move {
///         let ask = Alert::new("Delete “report.csv”?", "This can't be undone.").button("Delete").button("Cancel").destructive(0);
///         if alert_async(&window, ask).await == Some(0) {
///             deleted.set(true);
///         }
///     });
/// }
/// # let _ = delete;
/// ```
pub fn alert_async(window: &NSWindow, alert: Alert) -> crate::Answer<Option<usize>> {
    let cell: Pending<Option<usize>> = std::rc::Rc::default();
    let c = cell.clone();
    let shown = show_alert(window, alert, move |i| {
        if let Some(r) = c.borrow_mut().take() {
            r.send(i);
        }
    });
    let (parent, sheet) = (window.retain(), shown.window());
    let (answer, responder) = crate::task::answer(move || end_sheet(&parent, &sheet));
    *cell.borrow_mut() = Some(responder);
    answer
}

/// Ends `sheet` if it's still `parent`'s sheet (a dropped [`Answer`](crate::Answer)).
fn end_sheet(parent: &NSWindow, sheet: &NSWindow) {
    if parent.attachedSheet().is_some_and(|s| std::ptr::eq(&*s, sheet)) {
        parent.endSheet(sheet);
    }
}

fn show_alert(window: &NSWindow, alert: Alert, on_response: impl FnOnce(Option<usize>) + 'static) -> Retained<NSAlert> {
    let a = NSAlert::new(mtm());
    a.setMessageText(&NSString::from_str(&alert.title));
    a.setInformativeText(&NSString::from_str(&alert.message));
    a.setAlertStyle(if alert.warning { NSAlertStyle::Warning } else { NSAlertStyle::Informational });
    for (i, title) in alert.buttons.iter().enumerate() {
        let b = a.addButtonWithTitle(&NSString::from_str(title));
        if alert.destructive == Some(i) {
            b.setHasDestructiveAction(true);
        }
    }
    let respond = once(on_response);
    // A sheet ended without a button (its window closing, an abort) answers
    // as Cancel if there is one, and with `None` otherwise: never as the
    // first button, which may be the one that deletes.
    let (count, cancel) = (alert.buttons.len(), alert.buttons.iter().position(|b| b == "Cancel" || *b == crate::t("Cancel")));
    let block = RcBlock::new(move |r: NSModalResponse| {
        respond(usize::try_from(r - NSAlertFirstButtonReturn).ok().filter(|i| *i < count).or(cancel));
    });
    a.beginSheetModalForWindow_completionHandler(window, Some(&block));
    a
}

/// A sheet showing a view tree (see [`sheet`]). Closing it tears the tree
/// down.
#[derive(Clone)]
pub struct SheetHandle {
    parent: Retained<NSWindow>,
    sheet: Retained<NSWindow>,
}

impl SheetHandle {
    /// Closes the sheet (its `on_close` runs).
    pub fn close(&self) {
        self.parent.endSheet(&self.sheet);
    }

    /// The sheet's window.
    pub fn window(&self) -> Retained<NSWindow> {
        self.sheet.clone()
    }
}

/// Shows `content` (a [`Window`] built as usual: size and view) as a sheet on
/// `parent`. `on_close` runs when it closes; the view tree is then dropped.
pub fn sheet(parent: &NSWindow, content: Window, on_close: impl FnOnce() + 'static) -> SheetHandle {
    let sheet = objc2::rc::autoreleasepool(|_| content.open(mtm()));
    let handle = SheetHandle { parent: parent.retain(), sheet: sheet.clone() };
    let closed = once(move |()| on_close());
    let s = sheet.clone();
    let block = RcBlock::new(move |_r: NSModalResponse| {
        s.orderOut(None);
        // Views, undo history, delegates: as for a closed window.
        crate::teardown(&s);
        closed(());
    });
    parent.beginSheet_completionHandler(&sheet, Some(&block));
    handle
}

/// [`sheet`] for the window `make` builds: the signals `make` creates (the
/// sheet's fields) belong to the sheet and are freed when it closes, not
/// kept for the app's life as signals made in an action would be.
///
/// ```no_run
/// use ferrokit::*;
///
/// fn rename(parent: &objc2_app_kit::NSWindow) {
///     sheet_with(parent, || {
///         let name = Signal::new(String::new());
///         Window::new("Rename").size(300.0, 100.0).view(text_field(&name).padding(16.0))
///     }, || {});
/// }
/// # let _ = rename;
/// ```
pub fn sheet_with(parent: &NSWindow, make: impl FnOnce() -> crate::Window, on_close: impl FnOnce() + 'static) -> SheetHandle {
    let owner = crate::signal::Owner::new();
    let content = owner.with(make);
    let handle = sheet(parent, content, on_close);
    crate::keep_until_closed(&handle.sheet, owner);
    handle
}

/// The way a sheet from [`sheet_async`] answers: [`send`](Reply::send) a
/// value, or [`cancel`](Reply::cancel). Either closes the sheet. A cheap
/// `Clone` handle, for the sheet's buttons.
pub struct Reply<T> {
    inner: std::rc::Rc<ReplyInner<T>>,
}

struct ReplyInner<T> {
    value: RefCell<Option<T>>,
    handle: RefCell<Option<SheetHandle>>,
}

impl<T> Clone for Reply<T> {
    fn clone(&self) -> Self {
        Reply { inner: self.inner.clone() }
    }
}

impl<T> std::fmt::Debug for Reply<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reply").field("sent", &self.inner.value.borrow().is_some()).finish()
    }
}

impl<T> Reply<T> {
    /// Answers with `value` and closes the sheet (the first answer counts).
    pub fn send(&self, value: T) {
        {
            let mut v = self.inner.value.borrow_mut();
            if v.is_none() {
                *v = Some(value);
            }
        }
        self.close();
    }

    /// Closes the sheet with no answer: the future gives `None`.
    pub fn cancel(&self) {
        self.close();
    }

    fn close(&self) {
        let handle = self.inner.handle.borrow().clone();
        if let Some(h) = handle {
            end_sheet(&h.parent, &h.sheet);
        }
    }
}

/// Shows the window `make` builds as a sheet on `parent`, and returns a
/// future for its answer: `make` gets a [`Reply`] for the sheet's buttons
/// to send a value with (or cancel). `None` if it was cancelled or closed
/// another way; dropping the future (the task was cancelled) closes the
/// sheet. As with [`sheet_with`], the signals `make` creates belong to the
/// sheet.
///
/// ```no_run
/// use ferrokit::*;
///
/// fn rename(window: Retained<NSWindow>, title: Signal<String>) {
///     spawn_local(async move {
///         let name = sheet_async(&window, |reply: Reply<String>| {
///             let name = Signal::new(title.get_untracked());
///             let (ok, cancel) = (reply.clone(), reply);
///             Window::new("Rename").size(300.0, 110.0).view(
///                 vstack((
///                     text_field(&name),
///                     hstack((spacer(), button("Cancel", move || cancel.cancel()), button("Rename", move || ok.send(name.get_untracked())))),
///                 ))
///                 .spacing(12.0)
///                 .padding(16.0),
///             )
///         })
///         .await;
///         if let Some(name) = name {
///             title.set(name);
///         }
///     });
/// }
/// # let _ = rename;
/// ```
pub fn sheet_async<T: 'static>(parent: &NSWindow, make: impl FnOnce(Reply<T>) -> crate::Window) -> crate::Answer<Option<T>> {
    let inner = std::rc::Rc::new(ReplyInner { value: RefCell::new(None), handle: RefCell::new(None) });
    let reply = Reply { inner: inner.clone() };
    let cell: Pending<Option<T>> = std::rc::Rc::default();
    let (c, i) = (cell.clone(), inner.clone());
    let handle = sheet_with(parent, move || make(reply), move || {
        let value = i.value.borrow_mut().take();
        // The views' closures (and their replies) go with the sheet.
        i.handle.borrow_mut().take();
        if let Some(r) = c.borrow_mut().take() {
            r.send(value);
        }
    });
    *inner.handle.borrow_mut() = Some(handle.clone());
    let (answer, responder) = crate::task::answer(move || end_sheet(&handle.parent, &handle.sheet));
    *cell.borrow_mut() = Some(responder);
    answer
}

use objc2::Message as _;

/// What the Open panel allows.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct OpenOptions {
    /// File name extensions to allow (e.g. `["csv", "txt"]`); empty: any.
    pub extensions: Vec<String>,
    /// Allow choosing several files.
    pub multiple: bool,
    /// Choose folders instead of files.
    pub directories: bool,
}

impl OpenOptions {
    /// Any one file.
    pub fn new() -> Self {
        Self::default()
    }

    /// Only files with one of these name extensions (`&["csv", "txt"]`).
    pub fn extensions(mut self, extensions: &[&str]) -> Self {
        self.extensions = extensions.iter().map(|e| e.to_string()).collect();
        self
    }

    /// Lets the user choose several files.
    pub fn multiple(mut self) -> Self {
        self.multiple = true;
        self
    }

    /// Folders instead of files.
    pub fn directories(mut self) -> Self {
        self.directories = true;
        self
    }
}

fn content_types(extensions: &[String]) -> Option<Retained<NSArray<objc2_uniform_type_identifiers::UTType>>> {
    if extensions.is_empty() {
        return None;
    }
    let types: Vec<_> = extensions
        .iter()
        .filter_map(|e| objc2_uniform_type_identifiers::UTType::typeWithFilenameExtension(&NSString::from_str(e)))
        .collect();
    Some(NSArray::from_retained_slice(&types))
}

/// Shows the Open panel as a sheet on `window`; `on_done` gets the chosen
/// paths, or `None` if cancelled.
pub fn open_panel(window: &NSWindow, options: OpenOptions, on_done: impl FnOnce(Option<Vec<PathBuf>>) + 'static) {
    let (panel, block) = open_panel_parts(options, on_done);
    panel.beginSheetModalForWindow_completionHandler(window, &block);
}

/// Shows the Open panel as a sheet on `window`, and returns a future for the
/// chosen paths (`None` if cancelled): [`open_panel`] for a
/// [`spawn_local`](crate::spawn_local) task. Dropping the future (the task
/// was cancelled) closes the panel.
pub fn open_panel_async(window: &NSWindow, options: OpenOptions) -> crate::Answer<Option<Vec<PathBuf>>> {
    let cell: Pending<Option<Vec<PathBuf>>> = std::rc::Rc::default();
    let c = cell.clone();
    let (panel, block) = open_panel_parts(options, move |paths| {
        if let Some(r) = c.borrow_mut().take() {
            r.send(paths);
        }
    });
    panel.beginSheetModalForWindow_completionHandler(window, &block);
    let (parent, sheet) = (window.retain(), Retained::into_super(Retained::into_super(panel)));
    let (answer, responder) = crate::task::answer(move || end_sheet(&parent, &sheet));
    *cell.borrow_mut() = Some(responder);
    answer
}

/// The Open panel as a window of its own (when there's no window to attach
/// it to, e.g. a document app with no documents open).
pub(crate) fn open_panel_alone(options: OpenOptions, on_done: impl FnOnce(Option<Vec<PathBuf>>) + 'static) {
    let (panel, block) = open_panel_parts(options, on_done);
    panel.beginWithCompletionHandler(&block);
}

type PanelDone = RcBlock<dyn Fn(NSModalResponse)>;

fn open_panel_parts(options: OpenOptions, on_done: impl FnOnce(Option<Vec<PathBuf>>) + 'static) -> (Retained<NSOpenPanel>, PanelDone) {
    let panel = NSOpenPanel::openPanel(mtm());
    panel.setAllowsMultipleSelection(options.multiple);
    panel.setCanChooseDirectories(options.directories);
    panel.setCanChooseFiles(!options.directories);
    if let Some(types) = content_types(&options.extensions) {
        panel.setAllowedContentTypes(&types);
    }
    let done = once(on_done);
    let p = panel.clone();
    let block = RcBlock::new(move |r: NSModalResponse| {
        let paths = (r == NSModalResponseOK)
            .then(|| p.URLs().iter().filter_map(|u| u.path()).map(|s| PathBuf::from(s.to_string())).collect());
        done(paths);
    });
    (panel, block)
}

/// Shows the Save panel as a sheet on `window`, proposing `name` (in
/// `directory`, if given); `on_done` gets the chosen path, or `None` if
/// cancelled. Nothing is written: saving is the app's job.
pub fn save_panel(window: &NSWindow, name: &str, directory: Option<&std::path::Path>, on_done: impl FnOnce(Option<PathBuf>) + 'static) {
    show_save_panel(window, name, directory, on_done);
}

/// Shows the Save panel as a sheet on `window`, and returns a future for the
/// chosen path (`None` if cancelled): [`save_panel`] for a
/// [`spawn_local`](crate::spawn_local) task. Dropping the future (the task
/// was cancelled) closes the panel.
///
/// ```no_run
/// use ferrokit::*;
///
/// fn export(window: Retained<NSWindow>, csv: String, status: Signal<String>) {
///     spawn_local(async move {
///         let Some(path) = save_panel_async(&window, "export.csv", None).await else { return };
///         match background(move || std::fs::write(&path, csv)).await {
///             Ok(()) => status.set("Exported".into()),
///             Err(e) => status.set(format!("Couldn't export: {e}")),
///         }
///     });
/// }
/// # let _ = export;
/// ```
pub fn save_panel_async(window: &NSWindow, name: &str, directory: Option<&std::path::Path>) -> crate::Answer<Option<PathBuf>> {
    let cell: Pending<Option<PathBuf>> = std::rc::Rc::default();
    let c = cell.clone();
    let panel = show_save_panel(window, name, directory, move |path| {
        if let Some(r) = c.borrow_mut().take() {
            r.send(path);
        }
    });
    let (parent, sheet) = (window.retain(), Retained::into_super(panel));
    let (answer, responder) = crate::task::answer(move || end_sheet(&parent, &sheet));
    *cell.borrow_mut() = Some(responder);
    answer
}

fn show_save_panel(window: &NSWindow, name: &str, directory: Option<&std::path::Path>, on_done: impl FnOnce(Option<PathBuf>) + 'static) -> Retained<NSSavePanel> {
    let panel = NSSavePanel::savePanel(mtm());
    panel.setNameFieldStringValue(&NSString::from_str(name));
    if let Some(dir) = directory {
        let url = NSURL::fileURLWithPath(&NSString::from_str(&dir.to_string_lossy()));
        panel.setDirectoryURL(Some(&url));
    }
    let done = once(on_done);
    let p = panel.clone();
    let block = RcBlock::new(move |r: NSModalResponse| {
        let path = (r == NSModalResponseOK).then(|| p.URL()).flatten().and_then(|u| u.path()).map(|s| PathBuf::from(s.to_string()));
        done(path);
    });
    panel.beginSheetModalForWindow_completionHandler(window, &block);
    panel
}
