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

use ferrokit_core::kept::{UiCell, UiLocal};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSApplication, NSApplicationDelegate, NSApplicationTerminateReply, NSWindow};
use objc2_foundation::{NSString, NSURL, NSUserDefaults};

use crate::l10n::{t, t_with};
use crate::chrome::{MenuItem, Modifier, ToolbarItem, dynamic_submenu, menu_item, menu_separator};
use crate::dialogs::{Alert, OpenOptions, alert, open_panel, save_panel};
use crate::signal::Owner;
use crate::signal::{Effect, Signal, batch, untrack};
use crate::view::{AnyView, View};
use crate::{Window, mtm};

type Reader<T> = Rc<dyn Fn(&Path) -> std::io::Result<T>>;
type Writer<T> = Rc<dyn Fn(&T, &Path) -> std::io::Result<Option<T>>>;
type DocView<T> = Rc<dyn Fn(&Document<T>) -> AnyView>;
type DocToolbar<T> = Rc<dyn Fn(&Document<T>) -> Vec<ToolbarItem>>;

/// One kind of document: its file extensions, how to make an empty one,
/// read and write it, and the view of its window.
pub struct DocumentType<T: 'static> {
    extensions: Vec<String>,
    empty: Rc<dyn Fn() -> T>,
    read: Option<Reader<T>>,
    write: Option<Writer<T>>,
    view: DocView<T>,
    toolbar: Option<DocToolbar<T>>,
    size: (f64, f64),
    style: Option<Rc<dyn Fn(Window) -> Window>>,
    related: Vec<String>,
    /// Saves go to a new file that then replaces the old one.
    atomic: bool,
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
            toolbar: None,
            size: (640.0, 480.0),
            style: None,
            related: Vec::new(),
            atomic: false,
        }
    }

    /// Reads a file into a document's content (Open…, Open Recent, Revert).
    pub fn read(mut self, f: impl Fn(&Path) -> std::io::Result<T> + 'static) -> Self {
        self.read = Some(Rc::new(f));
        self
    }

    /// Writes a document's content to a file (Save, Save As…). Without it
    /// documents are read-only (a viewer).
    ///
    /// Saving over an existing file is safe: `f` is given a new file of
    /// the same name in a temporary folder, which replaces the document's
    /// file once `f` has returned `Ok`. A write that fails, or is cut
    /// short, leaves the file as it was. (So `f` mustn't count on the
    /// path's folder; the document's own is [`Document::path`].)
    pub fn write(mut self, f: impl Fn(&T, &Path) -> std::io::Result<()> + 'static) -> Self {
        self.write = Some(Rc::new(move |c, p| f(c, p).map(|()| None)));
        self.atomic = true;
        self
    }

    /// Like [`write`](Self::write), for documents whose content changes
    /// when it's saved: `f` writes the content and returns what the
    /// document holds afterwards, which becomes its saved state. A database
    /// with staged changes, say: saving commits them and returns the
    /// database with none staged.
    ///
    /// `f` is given the document's own file and changes it in place (a
    /// database is written by its engine, with its journal beside it),
    /// where [`write`](Self::write) replaces the file with a new one.
    pub fn write_returning(mut self, f: impl Fn(&T, &Path) -> std::io::Result<T> + 'static) -> Self {
        self.write = Some(Rc::new(move |c, p| f(c, p).map(Some)));
        self.atomic = false;
        self
    }

    /// Files beside each document that it reads and writes too, named by
    /// these suffixes (a SQLite database's `"-journal"`, `"-wal"`,
    /// `"-shm"`): the document holds access to them while it's open (see
    /// [`related_files`](crate::related_files)), before it's read and after
    /// Save As…. In the App Sandbox, declare them too (`related = [...]`
    /// in `cargo ferrokit`'s document type).
    pub fn related(mut self, suffixes: &[&str]) -> Self {
        self.related = suffixes.iter().map(|s| s.to_string()).collect();
        self
    }

    /// A toolbar for each document window, built with the window's view.
    pub fn toolbar(mut self, items: impl Fn(&Document<T>) -> Vec<ToolbarItem> + 'static) -> Self {
        self.toolbar = Some(Rc::new(items));
        self
    }

    /// The size of new document windows (default 640 × 480).
    pub fn size(mut self, width: f64, height: f64) -> Self {
        self.size = (width, height);
        self
    }

    /// Styles each document window: `style` gets the window as the
    /// document type made it and returns it with [`Window`]'s chrome
    /// methods applied, e.g.
    /// `.window(|w| w.full_size_content().toolbar_style(ToolbarStyle::Unified))`.
    pub fn window(mut self, style: impl Fn(Window) -> Window + 'static) -> Self {
        self.style = Some(Rc::new(style));
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
    /// Access to the files beside the document (its type's `related`).
    related: RefCell<Option<crate::RelatedFiles>>,
    /// Owns the document's signals and those its view makes outside the
    /// window's tree; they're freed with the document.
    _owner: Owner,
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
            None if self.0.untitled <= 1 => t("Untitled"),
            None => t_with("Untitled {n}", &[("n", &self.0.untitled)]),
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
    fn keep_related(&self, related: Option<crate::RelatedFiles>);
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
        let related = self.0.kind.claim(path);
        let kept = if self.0.kind.atomic { replacing(path, |to| write(&content, to))? } else { write(&content, path)? };
        if related.is_some() {
            *self.0.related.borrow_mut() = related;
        }
        let content = kept.unwrap_or(content);
        *self.0.saved.borrow_mut() = content.clone();
        batch(|| {
            self.0.content.set_if_changed(content);
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
    fn keep_related(&self, related: Option<crate::RelatedFiles>) {
        *self.0.related.borrow_mut() = related;
    }
}

impl<T> DocumentType<T> {
    /// Access to the files beside `path`, if the type has any.
    fn claim(&self, path: &Path) -> Option<crate::RelatedFiles> {
        let suffixes: Vec<&str> = self.related.iter().map(String::as_str).collect();
        (!suffixes.is_empty()).then(|| crate::related_files(path, &suffixes))
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
        let related = self.claim(path);
        let content = read(path)?;
        note_recent(path);
        let window = show(self.clone(), content, Some(path.to_path_buf()), 0);
        if let Some(doc) = document_of(&window) {
            doc.keep_related(related);
        }
        Ok(window)
    }

    fn extensions(&self) -> Vec<String> {
        self.extensions.clone()
    }

    fn can_open(&self) -> bool {
        self.read.is_some()
    }
}

thread_local! {
    static DOCS: UiCell<Vec<Rc<dyn AnyDoc>>> = const { UiCell::new(Vec::new()) };
    static OPENER: UiCell<Option<Rc<dyn Opener>>> = const { UiCell::new(None) };
    static UNTITLED: Cell<u32> = const { Cell::new(0) };
    static DELEGATE: UiCell<Option<Retained<AppDelegate>>> = const { UiCell::new(None) };
    /// The untitled document opened at launch, until files are opened.
    static AUTO_UNTITLED: UiCell<Weak<NSWindow>> = UiCell::new(Weak::default());
}

/// Opens a window for a document with `content`.
fn show<T: Clone + PartialEq + 'static>(kind: Rc<DocumentType<T>>, content: T, path: Option<PathBuf>, untitled: u32) -> Retained<NSWindow> {
    let owner = Owner::new();
    let (content_signal, edited_signal, path) = owner.with(|| (Signal::new(content.clone()), Signal::new(false), Signal::new(path)));
    let doc = Document(Rc::new(DocInner {
        saved: RefCell::new(content),
        content: content_signal,
        edited: edited_signal,
        path,
        untitled,
        kind: kind.clone(),
        window: RefCell::new(Weak::default()),
        closing: Cell::new(false),
        effects: RefCell::default(),
        related: RefCell::new(None),
        _owner: owner.clone(),
    }));
    // The document's own effects hold it weakly: it's freed (and its
    // signals with it) once its window and the views' handles are gone.
    let saved = Rc::downgrade(&doc.0);
    let edited = Effect::new(move || {
        let Some(d) = saved.upgrade() else { return };
        let differs = content_signal.with(|c| *c != *d.saved.borrow());
        edited_signal.set_if_changed(differs);
    });
    let view = owner.with(|| (kind.view)(&doc));
    // Files of the app's kinds dropped on a document window open.
    let extensions: Vec<&str> = kind.extensions.iter().map(String::as_str).collect();
    let mut spec = Window::new(&untrack(|| doc.title())).size(kind.size.0, kind.size.1).view(view).on_drop_files(&extensions, |paths| {
        for path in paths {
            if let Err(e) = open_document(&path) {
                eprintln!("ferrokit: couldn't open {}: {e}", path.display());
            }
        }
    });
    if let Some(toolbar) = &kind.toolbar {
        spec = spec.toolbar(owner.with(|| toolbar(&doc)));
    }
    if let Some(style) = &kind.style {
        spec = style(spec);
    }
    let window = crate::open(spec);
    // A document with a file comes back where its window last was.
    if let Some(p) = doc.path() {
        window.setFrameUsingName(&frame_name(&p));
    }
    // A document is open: the welcome window has done its job.
    close_welcome();
    *doc.0.window.borrow_mut() = Weak::from_retained(&window);
    // The title bar follows the file and the edited state (the dot in the
    // close button, the proxy icon of the file).
    let (d, w) = (Rc::downgrade(&doc.0), Weak::from_retained(&window));
    let title = Effect::new(move || {
        let Some(d) = d.upgrade().map(Document) else { return };
        let (name, path, edited) = (d.title(), d.path(), d.is_edited());
        if let Some(w) = w.load() {
            w.setTitle(&NSString::from_str(&name));
            w.setDocumentEdited(edited);
            // Its frame is remembered under its file (once it has one).
            if let Some(name) = path.as_deref().map(frame_name).filter(|n| w.frameAutosaveName().to_string() != n.to_string()) {
                w.setFrameAutosaveName(&name);
            }
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

/// Runs `write` on a new file that then takes the place of the one at
/// `path`, so a failed or interrupted write leaves that one whole. The new
/// file is made where the system keeps replacements for `path`'s volume
/// (reachable in the App Sandbox too); where there is no file yet, or no
/// such place, `write` gets `path` itself.
fn replacing<R>(path: &Path, write: impl FnOnce(&Path) -> std::io::Result<R>) -> std::io::Result<R> {
    use objc2_foundation::{NSFileManager, NSFileManagerItemReplacementOptions, NSSearchPathDirectory, NSSearchPathDomainMask};
    let Some(name) = path.file_name().filter(|_| path.exists()) else { return write(path) };
    let files = NSFileManager::defaultManager();
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    let folder = files.URLForDirectory_inDomain_appropriateForURL_create_error(NSSearchPathDirectory::ItemReplacementDirectory, NSSearchPathDomainMask::UserDomainMask, Some(&url), true);
    let Some(folder_path) = folder.as_ref().ok().and_then(|f| f.path()).map(|p| PathBuf::from(p.to_string())) else { return write(path) };
    let new = folder_path.join(name);
    let result = write(&new).and_then(|kept| {
        let new_url = NSURL::fileURLWithPath(&NSString::from_str(&new.to_string_lossy()));
        files
            .replaceItemAtURL_withItemAtURL_backupItemName_options_resultingItemURL_error(&url, &new_url, None, NSFileManagerItemReplacementOptions::empty(), None)
            .map(|()| kept)
            .map_err(|e| std::io::Error::other(e.localizedDescription().to_string()))
    });
    if let Ok(folder) = &folder {
        let _ = files.removeItemAtURL_error(folder);
    }
    result
}

/// The document of the frontmost document window.
fn current() -> Option<Rc<dyn AnyDoc>> {
    let app = NSApplication::sharedApplication(mtm());
    // The key window's document; under a sheet, its parent's; with a panel
    // or Settings key, the main window's.
    let key = app.keyWindow();
    let of_key = key.as_ref().and_then(|w| document_of(w).or_else(|| w.sheetParent().and_then(|p| document_of(&p))));
    if of_key.is_some() {
        return of_key;
    }
    let main = app.mainWindow();
    if let Some(d) = main.as_ref().and_then(|w| document_of(w)) {
        return Some(d);
    }
    if key.is_some() || main.is_some() {
        // A window that isn't a document's is in front (Settings alone).
        return None;
    }
    // Inactive app: the last document window opened that's visible.
    DOCS.with_borrow(|docs| docs.iter().rev().find(|d| d.window().is_some_and(|w| w.isVisible())).cloned())
}

/// Drops a closed window's document (called from window teardown).
pub(crate) fn forget(window: &NSWindow) {
    DOCS.with_borrow_mut(|docs| docs.retain(|d| d.window().is_some_and(|w| !std::ptr::eq(&*w, window))));
}

/// The name a document's window frame is remembered under.
fn frame_name(path: &Path) -> Retained<NSString> {
    NSString::from_str(&format!("Ferrokit document {}", path.display()))
}

/// Where the documents open at quit are remembered.
const OPEN_KEY: &str = "FerrokitOpenDocuments";

/// Whether the user keeps windows when quitting: System Settings ›
/// Desktop & Dock › "Close windows when quitting an application" off (the
/// `NSQuitAlwaysKeepsWindows` default). Apps reopen what was open then.
fn keeps_windows() -> bool {
    NSUserDefaults::standardUserDefaults().boolForKey(&NSString::from_str("NSQuitAlwaysKeepsWindows"))
}

/// Remembers the documents open now (those with files), for the next
/// launch: the app is quitting.
pub(crate) fn remember_open() {
    let paths: Vec<Retained<NSString>> = DOCS.with_borrow(|docs| docs.iter().filter_map(|d| d.path()).map(|p| NSString::from_str(&p.to_string_lossy())).collect());
    let list = objc2_foundation::NSArray::from_retained_slice(&paths);
    // SAFETY: an array of strings is a property list.
    unsafe { NSUserDefaults::standardUserDefaults().setObject_forKey(Some(&list), &NSString::from_str(OPEN_KEY)) };
}

/// At launch: reopens the documents open when the app last quit, if the
/// user keeps windows (see [`keeps_windows`]). How many opened.
pub(crate) fn restore_open() -> usize {
    if !keeps_windows() {
        return 0;
    }
    let Some(list) = NSUserDefaults::standardUserDefaults().stringArrayForKey(&NSString::from_str(OPEN_KEY)) else { return 0 };
    list.iter().map(|p| PathBuf::from(p.to_string())).filter(|p| {
        reach(p);
        p.exists()
    }).filter(|p| open_document(p).is_ok()).count()
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

/// How to make the welcome window, and the window while it's open.
type Welcome = (Option<Rc<dyn Fn() -> Window>>, Weak<NSWindow>);

thread_local! {
    static WELCOME: UiCell<Welcome> = UiCell::new((None, Weak::default()));
}

pub(crate) fn set_welcome(make: Rc<dyn Fn() -> Window>) {
    WELCOME.with_borrow_mut(|w| w.0 = Some(make));
}

/// Shows the app's welcome window (see [`App::welcome`](crate::App::welcome)),
/// opening it again if it was closed. False if the app has none.
pub fn show_welcome() -> bool {
    // (A closed window's views are gone: it's made again.)
    let (make, open) = WELCOME.with_borrow(|w| (w.0.clone(), w.1.load().filter(|w| w.isVisible())));
    if let Some(w) = open {
        w.makeKeyAndOrderFront(None);
        return true;
    }
    let Some(make) = make else { return false };
    // (Its builder's signals go with it when it closes.)
    let w = crate::open_with(move || make());
    w.center();
    w.makeKeyAndOrderFront(None);
    WELCOME.with_borrow_mut(|s| s.1 = Weak::from_retained(&w));
    true
}

/// The welcome window, while it's open.
pub fn welcome_window() -> Option<Retained<NSWindow>> {
    WELCOME.with_borrow(|w| w.1.load()).filter(|w| w.isVisible())
}

fn close_welcome() {
    if let Some(w) = WELCOME.with_borrow_mut(|w| std::mem::take(&mut w.1)).load() {
        w.close();
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
    // Sandboxed, a file from an earlier run is reached through its bookmark.
    reach(path);
    let window = opener.open(path)?;
    // However the file was opened (the Finder, Open…, the app's own code),
    // the untouched untitled window the app started with goes.
    close_auto_untitled();
    Ok(window)
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

const RECENTS_KEY: &str = "ferrokit recent documents";
const RECENTS_MAX: usize = 10;

/// The recently opened or saved documents, newest first (that still exist).
pub fn recent_documents() -> Vec<PathBuf> {
    // Read and written straight through (not a stored signal, whose writes
    // wait for the end of a batch: two opens in one batch would race).
    let list = NSUserDefaults::standardUserDefaults().stringForKey(&NSString::from_str(RECENTS_KEY));
    // (Sandboxed, a file is reached through its bookmark before it can be seen.)
    list.map(|l| l.to_string()).unwrap_or_default().lines().map(PathBuf::from).filter(|p| {
        reach(p);
        p.exists()
    }).collect()
}

fn set_recents(list: &[PathBuf]) {
    let joined = list.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>().join("\n");
    // SAFETY: an NSString value for a string key.
    unsafe { NSUserDefaults::standardUserDefaults().setObject_forKey(Some(&NSString::from_str(&joined)), &NSString::from_str(RECENTS_KEY)) };
}

fn note_recent(path: &Path) {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    remember_bookmark(&path);
    let mut list: Vec<PathBuf> = recent_documents().into_iter().filter(|p| *p != path).collect();
    list.insert(0, path);
    list.truncate(RECENTS_MAX);
    set_recents(&list);
}

// ---- the App Sandbox: reaching files again ----

/// Whether the app runs in the App Sandbox (its container is set).
fn sandboxed() -> bool {
    std::env::var_os("APP_SANDBOX_CONTAINER_ID").is_some()
}

fn bookmark_key(path: &Path) -> Retained<NSString> {
    NSString::from_str(&format!("Ferrokit bookmark {}", path.display()))
}

thread_local! {
    /// Files reached through their bookmarks (kept reachable while the app runs).
    static REACHED: UiCell<Vec<(PathBuf, Retained<NSURL>)>> = const { UiCell::new(Vec::new()) };
}

/// In the sandbox, the app reaches a file the user chose only while it
/// runs; a security-scoped bookmark, kept here, reaches it again after a
/// relaunch (Open Recent, reopening at launch).
fn remember_bookmark(path: &Path) {
    if !sandboxed() {
        return;
    }
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    if let Ok(data) = url.bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(objc2_foundation::NSURLBookmarkCreationOptions::WithSecurityScope, None, None) {
        // SAFETY: NSData is a property list value.
        unsafe { NSUserDefaults::standardUserDefaults().setObject_forKey(Some(&data), &bookmark_key(path)) };
    }
}

/// Before opening `path` in the sandbox: reaches it through its bookmark,
/// if one was kept. Whether it was (outside the sandbox, nothing to do).
pub(crate) fn reach(path: &Path) -> bool {
    if !sandboxed() || REACHED.with_borrow(|r| r.iter().any(|(p, _)| p == path)) {
        return true;
    }
    let Some(data) = NSUserDefaults::standardUserDefaults().dataForKey(&bookmark_key(path)) else { return false };
    let mut stale = objc2::runtime::Bool::NO;
    // SAFETY: a bookmark this app made; `stale` outlives the call.
    let url = unsafe { NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error(&data, objc2_foundation::NSURLBookmarkResolutionOptions::WithSecurityScope, None, &mut stale) };
    let Ok(url) = url else { return false };
    // SAFETY: balanced when the app quits (the system ends access then).
    if !unsafe { url.startAccessingSecurityScopedResource() } {
        return false;
    }
    if stale.as_bool() {
        remember_bookmark(path);
    }
    REACHED.with_borrow_mut(|r| r.push((path.to_path_buf(), url)));
    true
}

/// Keeps the app's access to `path` (a folder or file the user chose in an
/// Open panel, or dropped) for later launches: in the App Sandbox the app
/// reaches it only while it runs, and this keeps a security-scoped bookmark
/// to it in the app's defaults. Call [`regain_access`] for it after a
/// relaunch, before reading it. Does nothing outside the sandbox (or for a
/// path the app can't reach now). Documents opened through
/// [`DocumentType`](crate::DocumentType) are kept already.
pub fn remember_access(path: &Path) {
    remember_bookmark(path);
}

/// Reaches `path` again in the App Sandbox through the bookmark
/// [`remember_access`] kept, until the app quits (a folder's contents
/// too). Whether the app can reach it: `true` outside the sandbox, `false`
/// when no bookmark was kept or it no longer resolves (the folder was
/// deleted): ask the user to choose it again.
pub fn regain_access(path: &Path) -> bool {
    reach(path)
}

fn clear_recents() {
    set_recents(&[]);
}

// ---- the File menu ----

/// The File menu's document items: those above Close, and those after it.
pub(crate) fn file_items() -> (Vec<MenuItem>, Vec<MenuItem>) {
    let can_open = || OPENER.with_borrow(|o| o.as_ref().is_some_and(|o| o.can_open()));
    let mut above = vec![menu_item(t("New"), || {
        new_document();
    })
    .key("n")];
    if can_open() {
        above.push(menu_item(t("Open…"), choose_documents).key("o"));
        above.push(dynamic_submenu(&t("Open Recent"), || {
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
            items.push(menu_item(t("Clear Menu"), clear_recents).disabled(move || !(any)));
            items
        }));
    }
    let writable = || current().is_some_and(|d| d.writable());
    let after = vec![
        menu_item(t("Save"), || {
            if let Some(d) = current() {
                save_flow(d, |_| {});
            }
        })
        .key("s")
        .disabled(move || !writable()),
        menu_item(t("Save As…"), || {
            if let Some(d) = current() {
                save_as_flow(d, |_| {});
            }
        })
        .key_with("s", &[Modifier::Shift])
        .disabled(move || !writable()),
        menu_item(t("Revert to Saved"), confirm_revert).disabled(|| !current().is_some_and(|d| d.edited() && d.path().is_some())),
    ];
    (above, after)
}

/// Shows the Open panel for the app's document types, as File ▸ Open…
/// does (a button on a welcome window, say), and opens what's chosen.
pub fn choose_documents() {
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
        report(None, &t_with("The document “{name}” couldn't be opened.", &[("name", &name)]), &e.to_string());
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
            let ok = doc.save_to(&path).map_err(|e| report(doc.window().as_deref(), &t_with("The document “{name}” couldn't be saved.", &[("name", &doc.title())]), &e.to_string()));
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
            let ok = doc.save_to(&path).map_err(|e| report(doc.window().as_deref(), &t_with("The document “{name}” couldn't be saved.", &[("name", &doc.title())]), &e.to_string()));
            done(ok.is_ok());
        }
        None => done(false),
    });
}

fn confirm_revert() {
    let Some(doc) = current() else { return };
    let Some(window) = doc.window() else { return };
    let question = Alert::new(&t_with("Revert to the saved version of “{name}”?", &[("name", &doc.title())]), &t("Your current changes will be lost."))
        .button(&t("Revert"))
        .button(&t("Cancel"));
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
    let question = Alert::new(&t_with("Do you want to save the changes made to the document “{name}”?", &[("name", &doc.title())]), &t("Your changes will be lost if you don't save them."))
        .button(&t("Save"))
        .button(&t("Cancel"))
        .button(&t("Don't Save"));
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
    #[name = "FerrokitAppDelegate"]
    struct AppDelegate;

    unsafe impl NSObjectProtocol for AppDelegate {}

    // (Not NSApplicationDelegate's: QLPreviewPanelController's, an informal
    // protocol.)
    impl AppDelegate {
        // The Quick Look panel looks along the responder chain, which ends
        // here, for what controls it ([`quick_look`](crate::quick_look)).
        #[unsafe(method(acceptsPreviewPanelControl:))]
        fn accepts_preview(&self, _panel: &AnyObject) -> bool {
            crate::failure::guard("the app delegate (accepts_preview)", false, || {
                crate::quicklook::source().is_some()
            })
        }

        #[unsafe(method(beginPreviewPanelControl:))]
        fn begin_preview(&self, panel: &AnyObject) {
            crate::failure::guard("the app delegate (begin_preview)", (), || {
                if let Some(source) = crate::quicklook::source() {
                    // SAFETY: the panel holds its data source weakly; it's kept by quicklook.
                    let _: () = unsafe { msg_send![panel, setDataSource: &*source] };
                }
            })
        }

        #[unsafe(method(endPreviewPanelControl:))]
        fn end_preview(&self, _panel: &AnyObject) {
            crate::failure::guard("the app delegate (end_preview)", (), || {
            })
        }
    }

    unsafe impl NSApplicationDelegate for AppDelegate {
        // Files opened from the Finder (double-click, drop on the Dock icon,
        // `open file.txt`), at launch or later.
        #[unsafe(method(application:openURLs:))]
        fn open_urls(&self, _app: &NSApplication, urls: &objc2_foundation::NSArray<NSURL>) {
            crate::failure::guard("the app delegate (open_urls)", (), || {
                let paths: Vec<PathBuf> = urls.iter().filter_map(|u| u.path()).map(|p| PathBuf::from(p.to_string())).collect();
                // After launch work is done (the run loop is up).
                crate::later(move || {
                    for p in &paths {
                        open_reporting(p);
                    }
                });
            })
        }

        // The Dock icon's menu (App::dock_menu), built when it's opened.
        #[unsafe(method_id(applicationDockMenu:))]
        fn dock_menu(&self, _app: &NSApplication) -> Option<Retained<objc2_app_kit::NSMenu>> {
            crate::failure::guard("the app delegate (dock_menu)", None, || {
                let items = DOCK_MENU.with_borrow(Clone::clone);
                items.map(|items| {
                    let (menu, targets) = crate::chrome::build_menu(MainThreadMarker::from(self), items());
                    DOCK_TARGETS.with_borrow_mut(|t| *t = targets);
                    menu
                })
            })
        }

        #[unsafe(method(applicationShouldTerminate:))]
        fn should_terminate(&self, _app: &NSApplication) -> NSApplicationTerminateReply {
            crate::failure::guard("the app delegate (should_terminate)", NSApplicationTerminateReply::TerminateCancel, || {
                // What's open now comes back next time (if the user keeps windows).
                remember_open();
                if !DOCS.with_borrow(|docs| docs.iter().any(|d| d.edited())) {
                    return NSApplicationTerminateReply::TerminateNow;
                }
                // Ask outside this callback; AppKit waits for the reply.
                crate::later(review_and_quit);
                NSApplicationTerminateReply::TerminateLater
            })
        }
    }
);

/// Builds the Dock menu's items when it opens.
pub(crate) type DockItems = Rc<dyn Fn() -> Vec<crate::MenuItem>>;

thread_local! {
    /// The Dock menu's items ([`App::dock_menu`](crate::App::dock_menu)).
    pub(crate) static DOCK_MENU: UiCell<Option<DockItems>> = const { UiCell::new(None) };
    /// The last Dock menu's action targets, alive while it can be used.
    static DOCK_TARGETS: UiCell<Vec<Retained<NSObject>>> = const { UiCell::new(Vec::new()) };
}

/// Makes the app a document app for `kind` (see
/// [`App::documents`](crate::App::documents)): installs the app delegate.
pub(crate) fn install<T: Clone + PartialEq + 'static>(kind: DocumentType<T>) {
    let opener: Rc<dyn Opener> = Rc::new(Rc::new(kind));
    OPENER.with_borrow_mut(|o| *o = Some(opener));
    install_delegate();
}

/// Installs the app delegate (documents, the Dock menu), once.
pub(crate) fn install_delegate() {
    if DELEGATE.with_borrow(Option::is_some) {
        return;
    }
    let mtm = MainThreadMarker::new().expect("main thread");
    let delegate = AppDelegate::alloc(mtm).set_ivars(());
    // SAFETY: NSObject's designated initialiser.
    let delegate: Retained<AppDelegate> = unsafe { msg_send![super(delegate), init] };
    NSApplication::sharedApplication(mtm).setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    // Weak in AppKit: kept here.
    DELEGATE.with_borrow_mut(|d| *d = Some(delegate));
}
