//! Document apps: one window per file, with New, Open…, Open Recent, Save,
//! Save As… and Revert to Saved in the File menu, the window's edited dot,
//! a save-changes sheet on close, and a review of unsaved documents on quit.
//!
//! A [`DocumentType`] says how to read, write and show one kind of document;
//! [`App::documents`](crate::App::documents) makes the app a document app.
//! A [`Document`] is the content (a signal) plus its file. It's edited while
//! the content differs from what was last saved, so undoing back to the
//! saved text clears the dot, as in other Mac apps. Undo is per document:
//! each window has its own history.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use objc2::rc::{Retained, Weak};
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSApplication, NSApplicationDelegate, NSApplicationTerminateReply, NSWindow};
use objc2_foundation::{NSString, NSURL, NSUserDefaults};

use crate::chrome::{MenuItem, Modifier, dynamic_submenu, menu_item, menu_separator};
use crate::dialogs::{Alert, OpenOptions, alert, open_panel, save_panel};
use crate::signal::{Effect, Signal, batch, untrack};
use crate::view::{AnyView, View};
use crate::{Window, mtm};

type Reader<T> = Rc<dyn Fn(&Path) -> std::io::Result<T>>;
type Writer<T> = Rc<dyn Fn(&T, &Path) -> std::io::Result<()>>;
type DocView<T> = Rc<dyn Fn(&Document<T>) -> AnyView>;

/// One kind of document: its file extensions, how to make an empty one,
/// read and write it, and the view of its window.
pub struct DocumentType<T: 'static> {
    extensions: Vec<String>,
    empty: Rc<dyn Fn() -> T>,
    read: Option<Reader<T>>,
    write: Option<Writer<T>>,
    view: DocView<T>,
    size: (f64, f64),
}

impl<T: Clone + PartialEq + 'static> DocumentType<T> {
    /// A document type with these file extensions (the first is used for
    /// new files; Open… shows only these). `empty` is a new document's
    /// content and `view` builds a document's window content. Add
    /// [`read`](Self::read) to open files and [`write`](Self::write) to save
    /// them.
    pub fn new<V: View>(extensions: &[&str], empty: impl Fn() -> T + 'static, view: impl Fn(&Document<T>) -> V + 'static) -> Self {
        DocumentType {
            extensions: extensions.iter().map(|e| e.to_string()).collect(),
            empty: Rc::new(empty),
            read: None,
            write: None,
            view: Rc::new(move |d| AnyView::new(view(d))),
            size: (640.0, 480.0),
        }
    }

    /// Reads a file into a document's content (Open…, Open Recent, Revert).
    pub fn read(mut self, f: impl Fn(&Path) -> std::io::Result<T> + 'static) -> Self {
        self.read = Some(Rc::new(f));
        self
    }

    /// Writes a document's content to a file (Save, Save As…). Without it
    /// documents are read-only (a viewer).
    pub fn write(mut self, f: impl Fn(&T, &Path) -> std::io::Result<()> + 'static) -> Self {
        self.write = Some(Rc::new(f));
        self
    }

    /// The size of new document windows (default 640 × 480).
    pub fn size(mut self, width: f64, height: f64) -> Self {
        self.size = (width, height);
        self
    }
}

/// An open document: its content and its file. Cloning gives another handle.
pub struct Document<T: 'static>(Rc<DocInner<T>>);

impl<T> Clone for Document<T> {
    fn clone(&self) -> Self {
        Document(self.0.clone())
    }
}

struct DocInner<T: 'static> {
    content: Signal<T>,
    path: Signal<Option<PathBuf>>,
    edited: Signal<bool>,
    /// The content as last read or saved.
    saved: RefCell<T>,
    /// "Untitled", "Untitled 2"… for a document not yet saved.
    untitled: u32,
    kind: Rc<DocumentType<T>>,
    window: RefCell<Weak<NSWindow>>,
    /// Closing without asking (after the user chose).
    closing: Cell<bool>,
    effects: RefCell<Vec<Effect>>,
}

impl<T: Clone + PartialEq + 'static> Document<T> {
    /// The content. Change it (or bind a view to it) to edit the document.
    pub fn content(&self) -> &Signal<T> {
        &self.0.content
    }

    /// The file, once opened or saved.
    pub fn path(&self) -> Option<PathBuf> {
        self.0.path.get()
    }

    /// Whether the content differs from the file (or an empty document).
    pub fn is_edited(&self) -> bool {
        self.0.edited.get()
    }

    /// The name shown in the title bar: the file name, or "Untitled".
    pub fn title(&self) -> String {
        match self.0.path.get() {
            Some(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            None if self.0.untitled <= 1 => "Untitled".into(),
            None => format!("Untitled {}", self.0.untitled),
        }
    }

    /// The document's window, while it's open.
    pub fn window(&self) -> Option<Retained<NSWindow>> {
        self.0.window.borrow().load()
    }
}

/// What the File menu and the close and quit flows need from a document of
/// any type.
trait AnyDoc {
    fn window(&self) -> Option<Retained<NSWindow>>;
    fn path(&self) -> Option<PathBuf>;
    fn edited(&self) -> bool;
    fn writable(&self) -> bool;
    fn title(&self) -> String;
    fn extension(&self) -> String;
    fn save_to(&self, path: &Path) -> std::io::Result<()>;
    fn revert(&self) -> std::io::Result<()>;
    fn closing(&self) -> &Cell<bool>;
}

impl<T: Clone + PartialEq + 'static> AnyDoc for Document<T> {
    fn window(&self) -> Option<Retained<NSWindow>> {
        Document::window(self)
    }
    fn path(&self) -> Option<PathBuf> {
        self.0.path.get_untracked()
    }
    fn edited(&self) -> bool {
        self.0.edited.get_untracked()
    }
    fn writable(&self) -> bool {
        self.0.kind.write.is_some()
    }
    fn title(&self) -> String {
        untrack(|| Document::title(self))
    }
    fn extension(&self) -> String {
        self.0.kind.extensions.first().cloned().unwrap_or_default()
    }
    fn save_to(&self, path: &Path) -> std::io::Result<()> {
        let write = self.0.kind.write.as_ref().ok_or_else(|| std::io::Error::other("this kind of document can't be saved"))?;
        let content = self.0.content.get_untracked();
        write(&content, path)?;
        *self.0.saved.borrow_mut() = content;
        batch(|| {
            self.0.path.set(Some(path.to_path_buf()));
            self.0.edited.set_if_changed(false);
        });
        note_recent(path);
        Ok(())
    }
    fn revert(&self) -> std::io::Result<()> {
        let (Some(path), Some(read)) = (self.path(), self.0.kind.read.as_ref()) else {
            return Err(std::io::Error::other("the document has no file to revert to"));
        };
        let content = read(&path)?;
        *self.0.saved.borrow_mut() = content.clone();
        batch(|| {
            self.0.content.set(content);
            self.0.edited.set_if_changed(false);
        });
        Ok(())
    }
    fn closing(&self) -> &Cell<bool> {
        &self.0.closing
    }
}

/// Opens documents of the app's document type.
trait Opener {
    fn new_document(&self) -> Retained<NSWindow>;
    fn open(&self, path: &Path) -> std::io::Result<Retained<NSWindow>>;
    fn extensions(&self) -> Vec<String>;
    fn can_open(&self) -> bool;
}

impl<T: Clone + PartialEq + 'static> Opener for Rc<DocumentType<T>> {
    fn new_document(&self) -> Retained<NSWindow> {
        let n = UNTITLED.with(|u| u.replace(u.get() + 1) + 1);
        show(self.clone(), (self.empty)(), None, n)
    }

    fn open(&self, path: &Path) -> std::io::Result<Retained<NSWindow>> {
        let read = self.read.as_ref().ok_or_else(|| std::io::Error::other("this app can't open files"))?;
        let content = read(path)?;
        note_recent(path);
        Ok(show(self.clone(), content, Some(path.to_path_buf()), 0))
    }

    fn extensions(&self) -> Vec<String> {
        self.extensions.clone()
    }

    fn can_open(&self) -> bool {
        self.read.is_some()
    }
}

thread_local! {
    static DOCS: RefCell<Vec<Rc<dyn AnyDoc>>> = const { RefCell::new(Vec::new()) };
    static OPENER: RefCell<Option<Rc<dyn Opener>>> = const { RefCell::new(None) };
    static UNTITLED: Cell<u32> = const { Cell::new(0) };
    static DELEGATE: RefCell<Option<Retained<AppDelegate>>> = const { RefCell::new(None) };
    /// The untitled document opened at launch, until files are opened.
    static AUTO_UNTITLED: RefCell<Weak<NSWindow>> = RefCell::new(Weak::default());
}

/// Opens a window for a document with `content`.
fn show<T: Clone + PartialEq + 'static>(kind: Rc<DocumentType<T>>, content: T, path: Option<PathBuf>, untitled: u32) -> Retained<NSWindow> {
    let doc = Document(Rc::new(DocInner {
        saved: RefCell::new(content.clone()),
        content: Signal::new(content),
        edited: Signal::new(false),
        path: Signal::new(path),
        untitled,
        kind: kind.clone(),
        window: RefCell::new(Weak::default()),
        closing: Cell::new(false),
        effects: RefCell::default(),
    }));
    // Edited while the content differs from the saved content.
    let d = doc.clone();
    let edited = Effect::new(move || {
        let differs = d.0.content.with(|c| *c != *d.0.saved.borrow());
        d.0.edited.set_if_changed(differs);
    });
    let view = (kind.view)(&doc);
    let window = crate::open(Window::new(&untrack(|| doc.title())).size(kind.size.0, kind.size.1).view(view));
    *doc.0.window.borrow_mut() = Weak::from_retained(&window);
    // The title bar follows the file and the edited state (the dot in the
    // close button, the proxy icon of the file).
    let (d, w) = (doc.clone(), Weak::from_retained(&window));
    let title = Effect::new(move || {
        let (name, path, edited) = (d.title(), d.path(), d.is_edited());
        if let Some(w) = w.load() {
            w.setTitle(&NSString::from_str(&name));
            w.setDocumentEdited(edited);
            let url = path.map(|p| NSURL::fileURLWithPath(&NSString::from_str(&p.to_string_lossy())));
            w.setRepresentedURL(url.as_deref());
        }
    });
    doc.0.effects.borrow_mut().extend([edited, title]);
    DOCS.with_borrow_mut(|docs| docs.push(Rc::new(doc)));
    window
}

/// The document whose window is `window`.
fn document_of(window: &NSWindow) -> Option<Rc<dyn AnyDoc>> {
    DOCS.with_borrow(|docs| docs.iter().find(|d| d.window().is_some_and(|w| std::ptr::eq(&*w, window))).cloned())
}

/// The document of the frontmost document window.
fn current() -> Option<Rc<dyn AnyDoc>> {
    let app = NSApplication::sharedApplication(mtm());
    let front = app.keyWindow().or_else(|| app.mainWindow());
    front.and_then(|w| document_of(&w)).or_else(|| {
        // Inactive app: the last document window opened that's visible.
        DOCS.with_borrow(|docs| docs.iter().rev().find(|d| d.window().is_some_and(|w| w.isVisible())).cloned())
    })
}

/// Drops a closed window's document (called from window teardown).
pub(crate) fn forget(window: &NSWindow) {
    DOCS.with_borrow_mut(|docs| docs.retain(|d| d.window().is_some_and(|w| !std::ptr::eq(&*w, window))));
}

/// Opens the untitled document a document app starts with (when no files
/// were opened), remembering it: opening files closes it while it's
/// unchanged, as TextEdit does.
pub(crate) fn new_launch_document() {
    let window = new_document();
    AUTO_UNTITLED.with_borrow_mut(|a| *a = window.as_ref().map(Weak::from_retained).unwrap_or_default());
}

fn close_auto_untitled() {
    let Some(window) = AUTO_UNTITLED.with_borrow_mut(std::mem::take).load() else { return };
    let opened_more = DOCS.with_borrow(|docs| docs.len() > 1);
    if opened_more && !document_edited(&window) && document_of(&window).is_some_and(|d| d.path().is_none()) {
        window.close();
    }
}

/// Opens a new, empty document. `None` if the app has no document type.
pub fn new_document() -> Option<Retained<NSWindow>> {
    let opener = OPENER.with_borrow(Clone::clone)?;
    Some(opener.new_document())
}

/// Opens the document at `path`, or brings its window to the front if it's
/// already open.
pub fn open_document(path: &Path) -> std::io::Result<Retained<NSWindow>> {
    let open = DOCS.with_borrow(|docs| docs.iter().find(|d| d.path().as_deref() == Some(path)).and_then(|d| d.window()));
    if let Some(w) = open {
        w.makeKeyAndOrderFront(None);
        note_recent(path);
        return Ok(w);
    }
    let opener = OPENER.with_borrow(Clone::clone).ok_or_else(|| std::io::Error::other("the app has no document type"))?;
    opener.open(path)
}

/// Saves the document in `window` to `path` (Save As… without the panel);
/// it then belongs to that file.
pub fn save_document_as(window: &NSWindow, path: &Path) -> std::io::Result<()> {
    document_of(window).ok_or_else(|| std::io::Error::other("not a document window"))?.save_to(path)
}

/// Saves the document in `window` to its file. An error if it has none yet
/// (use [`save_document_as`], or the Save menu item, which asks).
pub fn save_document(window: &NSWindow) -> std::io::Result<()> {
    let doc = document_of(window).ok_or_else(|| std::io::Error::other("not a document window"))?;
    let path = doc.path().ok_or_else(|| std::io::Error::other("the document has no file yet"))?;
    doc.save_to(&path)
}

/// Reads the document in `window` back from its file, dropping its changes.
pub fn revert_document(window: &NSWindow) -> std::io::Result<()> {
    document_of(window).ok_or_else(|| std::io::Error::other("not a document window"))?.revert()
}

/// Whether the document in `window` has unsaved changes.
pub fn document_edited(window: &NSWindow) -> bool {
    document_of(window).is_some_and(|d| d.edited())
}

// ---- recent documents ----

const RECENTS_KEY: &str = "appkit-ui recent documents";
const RECENTS_MAX: usize = 10;

/// The recently opened or saved documents, newest first (that still exist).
pub fn recent_documents() -> Vec<PathBuf> {
    // Read and written straight through (not a stored signal, whose writes
    // wait for the end of a batch: two opens in one batch would race).
    let list = NSUserDefaults::standardUserDefaults().stringForKey(&NSString::from_str(RECENTS_KEY));
    list.map(|l| l.to_string()).unwrap_or_default().lines().map(PathBuf::from).filter(|p| p.exists()).collect()
}

fn set_recents(list: &[PathBuf]) {
    let joined = list.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>().join("\n");
    // SAFETY: an NSString value for a string key.
    unsafe { NSUserDefaults::standardUserDefaults().setObject_forKey(Some(&NSString::from_str(&joined)), &NSString::from_str(RECENTS_KEY)) };
}

fn note_recent(path: &Path) {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut list: Vec<PathBuf> = recent_documents().into_iter().filter(|p| *p != path).collect();
    list.insert(0, path);
    list.truncate(RECENTS_MAX);
    set_recents(&list);
}

fn clear_recents() {
    set_recents(&[]);
}

// ---- the File menu ----

/// The File menu's document items: those above Close, and those after it.
pub(crate) fn file_items() -> (Vec<MenuItem>, Vec<MenuItem>) {
    let can_open = || OPENER.with_borrow(|o| o.as_ref().is_some_and(|o| o.can_open()));
    let mut above = vec![menu_item("New", || {
        new_document();
    })
    .key("n")];
    if can_open() {
        above.push(menu_item("Open…", choose_and_open).key("o"));
        above.push(dynamic_submenu("Open Recent", || {
            let mut items: Vec<MenuItem> = recent_documents()
                .into_iter()
                .map(|p| {
                    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    menu_item(&name, move || open_reporting(&p))
                })
                .collect();
            let any = !items.is_empty();
            if any {
                items.push(menu_separator());
            }
            items.push(menu_item("Clear Menu", clear_recents).enabled_when(move || any));
            items
        }));
    }
    let writable = || current().is_some_and(|d| d.writable());
    let after = vec![
        menu_item("Save", || {
            if let Some(d) = current() {
                save_flow(d, |_| {});
            }
        })
        .key("s")
        .enabled_when(writable),
        menu_item("Save As…", || {
            if let Some(d) = current() {
                save_as_flow(d, |_| {});
            }
        })
        .key_with("s", &[Modifier::Shift])
        .enabled_when(writable),
        menu_item("Revert to Saved", confirm_revert).enabled_when(|| current().is_some_and(|d| d.edited() && d.path().is_some())),
    ];
    (above, after)
}

fn choose_and_open() {
    let Some(opener) = OPENER.with_borrow(Clone::clone) else { return };
    let app = NSApplication::sharedApplication(mtm());
    let options = OpenOptions { extensions: opener.extensions(), multiple: true, directories: false };
    let open_each = |paths: Option<Vec<PathBuf>>| paths.into_iter().flatten().for_each(|p| open_reporting(&p));
    match app.keyWindow().or_else(|| app.mainWindow()) {
        Some(w) => open_panel(&w, options, open_each),
        // No window to attach the panel to: a free-standing one.
        None => crate::dialogs::open_panel_alone(options, open_each),
    }
}

/// Opens `path`, telling the user if it can't be.
fn open_reporting(path: &Path) {
    if let Err(e) = open_document(path) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        report(None, &format!("The document “{name}” couldn't be opened."), &e.to_string());
    }
}

fn report(window: Option<&NSWindow>, title: &str, message: &str) {
    let app = NSApplication::sharedApplication(mtm());
    let window = window.map(objc2::Message::retain).or_else(|| app.keyWindow()).or_else(|| app.mainWindow());
    match window {
        Some(w) => alert(&w, Alert::new(title, message).warning(), |_| {}),
        None => eprintln!("{title} {message}"),
    }
}

/// Saves `doc`: to its file, or through the Save panel if it has none.
/// `done(true)` once saved.
fn save_flow(doc: Rc<dyn AnyDoc>, done: impl FnOnce(bool) + 'static) {
    match doc.path() {
        Some(path) => {
            let ok = doc.save_to(&path).map_err(|e| report(doc.window().as_deref(), &format!("The document “{}” couldn't be saved.", doc.title()), &e.to_string()));
            done(ok.is_ok());
        }
        None => save_as_flow(doc, done),
    }
}

/// Saves `doc` to a file chosen in the Save panel.
fn save_as_flow(doc: Rc<dyn AnyDoc>, done: impl FnOnce(bool) + 'static) {
    let Some(window) = doc.window() else { return done(false) };
    let name = match doc.path() {
        Some(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        None => format!("{}.{}", doc.title(), doc.extension()),
    };
    let dir = doc.path().and_then(|p| p.parent().map(Path::to_path_buf));
    save_panel(&window, &name, dir.as_deref(), move |path| match path {
        Some(path) => {
            let ok = doc.save_to(&path).map_err(|e| report(doc.window().as_deref(), &format!("The document “{}” couldn't be saved.", doc.title()), &e.to_string()));
            done(ok.is_ok());
        }
        None => done(false),
    });
}

fn confirm_revert() {
    let Some(doc) = current() else { return };
    let Some(window) = doc.window() else { return };
    let question = Alert::new(&format!("Revert to the saved version of “{}”?", doc.title()), "Your current changes will be lost.")
        .button("Revert")
        .button("Cancel");
    alert(&window, question, move |i| {
        if i == 0
            && let Err(e) = doc.revert()
        {
            report(doc.window().as_deref(), "The document couldn't be reverted.", &e.to_string());
        }
    });
}

// ---- closing and quitting ----

/// Asked by the window delegate before a window closes: an edited
/// document asks to save first (and closes itself afterwards).
pub(crate) fn window_should_close(window: &NSWindow) -> bool {
    let Some(doc) = document_of(window) else { return true };
    if !doc.edited() || doc.closing().get() {
        return true;
    }
    close_flow(doc, |_| {});
    false
}

/// Asks whether to save an edited document, then closes it unless the user
/// cancels. `done(true)` once it's closed.
fn close_flow(doc: Rc<dyn AnyDoc>, done: impl FnOnce(bool) + 'static) {
    let Some(window) = doc.window() else { return done(true) };
    window.makeKeyAndOrderFront(None);
    let question = Alert::new(&format!("Do you want to save the changes made to the document “{}”?", doc.title()), "Your changes will be lost if you don't save them.")
        .button("Save")
        .button("Cancel")
        .button("Don't Save");
    let close = move |doc: &Rc<dyn AnyDoc>| {
        doc.closing().set(true);
        if let Some(w) = doc.window() {
            w.close();
        }
    };
    alert(&window, question, move |i| match i {
        0 => {
            let d = doc.clone();
            save_flow(doc, move |saved| {
                if saved {
                    close(&d);
                }
                done(saved);
            });
        }
        2 => {
            close(&doc);
            done(true);
        }
        _ => done(false),
    });
}

/// On quit: each edited document asks in turn; any Cancel stops the quit.
fn review_and_quit() {
    let edited: Vec<Rc<dyn AnyDoc>> = DOCS.with_borrow(|docs| docs.iter().filter(|d| d.edited()).cloned().collect());
    fn next(mut rest: Vec<Rc<dyn AnyDoc>>) {
        let app = NSApplication::sharedApplication(mtm());
        let Some(doc) = rest.pop() else { return app.replyToApplicationShouldTerminate(true) };
        close_flow(doc, move |closed| {
            if closed {
                next(rest);
            } else {
                NSApplication::sharedApplication(mtm()).replyToApplicationShouldTerminate(false);
            }
        });
    }
    next(edited);
}

define_class!(
    // A document app's delegate: reviews unsaved documents before quitting.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AppKitUiAppDelegate"]
    struct AppDelegate;

    unsafe impl NSObjectProtocol for AppDelegate {}

    unsafe impl NSApplicationDelegate for AppDelegate {
        // Files opened from the Finder (double-click, drop on the Dock icon,
        // `open file.txt`), at launch or later.
        #[unsafe(method(application:openURLs:))]
        fn open_urls(&self, _app: &NSApplication, urls: &objc2_foundation::NSArray<NSURL>) {
            let paths: Vec<PathBuf> = urls.iter().filter_map(|u| u.path()).map(|p| PathBuf::from(p.to_string())).collect();
            // After launch work is done (the run loop is up).
            crate::later(move || {
                for p in &paths {
                    open_reporting(p);
                }
                close_auto_untitled();
            });
        }

        #[unsafe(method(applicationShouldTerminate:))]
        fn should_terminate(&self, _app: &NSApplication) -> NSApplicationTerminateReply {
            if !DOCS.with_borrow(|docs| docs.iter().any(|d| d.edited())) {
                return NSApplicationTerminateReply::TerminateNow;
            }
            // Ask outside this callback; AppKit waits for the reply.
            crate::later(review_and_quit);
            NSApplicationTerminateReply::TerminateLater
        }
    }
);

/// Makes the app a document app for `kind` (see
/// [`App::documents`](crate::App::documents)): installs the app delegate.
pub(crate) fn install<T: Clone + PartialEq + 'static>(kind: DocumentType<T>) {
    let opener: Rc<dyn Opener> = Rc::new(Rc::new(kind));
    OPENER.with_borrow_mut(|o| *o = Some(opener));
    let mtm = MainThreadMarker::new().expect("main thread");
    let delegate = AppDelegate::alloc(mtm).set_ivars(());
    // SAFETY: NSObject's designated initialiser.
    let delegate: Retained<AppDelegate> = unsafe { msg_send![super(delegate), init] };
    NSApplication::sharedApplication(mtm).setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    // Weak in AppKit: kept here.
    DELEGATE.with_borrow_mut(|d| *d = Some(delegate));
}
