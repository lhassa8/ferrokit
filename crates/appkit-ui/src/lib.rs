//! A native macOS UI framework for Rust: every view is a real AppKit view
//! (`NSTableView`, `NSTextView`, `NSSplitView`…), built once and updated in
//! place by fine-grained signals. No virtual DOM, no diffing, no web view.
//!
//! ```no_run
//! use appkit_ui::*;
//!
//! let count = Signal::new(0);
//! let (c1, c2) = (count.clone(), count.clone());
//! App::new("Counter")
//!     .window(Window::new("Counter").size(320.0, 140.0).view(
//!         vstack((
//!             label(move || format!("Clicked {} times", c1.get())),
//!             button("Click me", move || c2.update(|n| *n += 1)),
//!         ))
//!         .padding(20.0),
//!     ))
//!     .run();
//! ```
//!
//! - **State:** [`Signal`], [`Memo`], [`Effect`], [`batch`], [`spawn`] for
//!   background work, [`stored`] for settings that persist.
//! - **Views and layout:** [`vstack`], [`hstack`], [`grid`], [`spacer`],
//!   [`scroll`], [`for_each`] (keyed lists), [`dynamic`], and the
//!   [`ViewExt`] modifiers (padding, sizes, context menus, accessibility
//!   labels).
//! - **Controls:** [`text_field`], [`toggle`], [`checkbox`], [`picker`],
//!   [`slider`], [`stepper`], [`progress`], [`image`], [`button`].
//! - **Deep widgets:** [`table`] (millions of rows, sorting, editing, drag
//!   and drop), [`outline`] and [`sidebar`], [`text_editor`].
//! - **App shell:** menus ([`App::menu`]), toolbars, [`split`] views,
//!   [`sheet`]s, [`alert`]s, open and save panels, a Settings window,
//!   window restoration, undo ([`undoable`]), [`on_frame`] for live data.
//! - **Testing:** [`testing`] (self-checks, accessibility audits, window
//!   captures).
//!
//! The guide (`crates/appkit-ui/guide`) walks through all of it; its
//! examples are compiled as doc tests. macOS 15 or later. Main thread only.

#![warn(missing_docs)]

mod appkit_views;
mod chrome;
mod collection;
mod controls;
mod dialogs;
mod documents;
mod frame;
mod layout;
mod menu;
mod models;
mod outline;
mod platform;
mod signal;
mod store;
mod undo;
pub mod testing;
mod view;

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSResponder, NSView, NSWindowStyleMask,
};
use objc2_foundation::NSObject;

use platform::{Native, Platform};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

pub use appkit_host::{is_dark, system_colors};
pub use frame::{FrameLoop, on_frame};
pub use store::{Stored, stored};
pub use undo::{set_undoable, undoable, undoable_in};
pub use signal::{Effect, Memo, Signal, after_batch, batch, effect_runs, untrack};
pub use controls::{
    Color, ColorWell, ComboBox, DatePicker, SearchField, Segmented, color_well, combo_box, date_picker, search_field, segmented,
    Checkbox, Image, Picker, Progress, Slider, Stepper, TextField, checkbox, image, picker, progress, secure_field, slider,
    stepper, text_field, toggle,
};
pub use chrome::{MenuItem, Modifier, Split, dynamic_submenu, submenu, ToolbarItem, menu_item, menu_separator, split, toolbar_button, toolbar_search, toolbar_space};
pub use dialogs::{Alert, OpenOptions, SheetHandle, alert, open_panel, save_panel, sheet};
pub use documents::{
    Document, DocumentType, document_edited, new_document, open_document, recent_documents, revert_document, save_document, save_document_as,
};
pub use layout::layout_stats;
pub use models::{Table, TextEditor, table, text_editor};
pub use outline::{Outline, TreeNode, outline, sidebar};
pub use collection::{Collection, CollectionItem, ItemImage, collection, collection_cells_made};
pub use appkit_host::table::{CellImage, Column, ColumnState, Sort, TableModel};
pub use appkit_host::text::TextModel;
pub use appkit_host::ViewId;
/// The AppKit window type the window functions take and return.
pub use objc2_app_kit::NSWindow;
/// The owning pointer to an AppKit object (as returned by [`open`]).
pub use objc2::rc::Retained;
pub use view::{
    Align, AnyView, Axis, Cx, Dynamic, ForEach, Grid, Identifiable, Modified, Node, Size, Spacer, Stack, View, ViewExt, Views,
    dynamic, for_each, grid, hstack, live_nodes, spacer, vstack, watch, Watch,
};
pub use appkit_views::{Button, Divider, Label, Scroll, Text, button, divider, label, scroll, text};

fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("appkit-ui must be used on the main thread")
}

/// A window to open when the app starts.
pub struct Window {
    title: String,
    size: (f64, f64),
    content: Option<Box<dyn FnOnce() -> Retained<NSView>>>,
    view: Option<AnyView>,
    toolbar: Vec<ToolbarItem>,
    autosave: Option<String>,
    fixed: bool,
}

impl Window {
    /// A titled, resizable window of 800 × 600 points.
    pub fn new(title: &str) -> Self {
        Window {
            title: title.into(),
            size: (800.0, 600.0),
            content: None,
            view: None,
            toolbar: Vec::new(),
            autosave: None,
            fixed: false,
        }
    }

    /// The content size in points.
    pub fn size(mut self, width: f64, height: f64) -> Self {
        self.size = (width, height);
        self
    }

    /// The window's content: `view`, laid out to fill the window.
    pub fn view(mut self, view: impl View) -> Self {
        self.view = Some(AnyView::new(view));
        self
    }

    /// A unified toolbar with these items.
    pub fn toolbar(mut self, items: Vec<ToolbarItem>) -> Self {
        self.toolbar = items;
        self
    }

    /// Saves the window's position and size under `name` whenever they
    /// change, and reopens it there next time (AppKit's frame autosave, in
    /// the app's user defaults).
    pub fn remember_frame(mut self, name: &str) -> Self {
        self.autosave = Some(name.into());
        self
    }

    /// A plain `NSView` as the window's content (escape hatch; no layout).
    pub fn content(mut self, build: impl FnOnce() -> Retained<NSView> + 'static) -> Self {
        self.content = Some(Box::new(build));
        self
    }

    pub(crate) fn open(self, mtm: MainThreadMarker) -> Retained<NSWindow> {
        let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(self.size.0, self.size.1));
        let style = if self.fixed {
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable
        } else {
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Miniaturizable | NSWindowStyleMask::Resizable
        };
        // SAFETY: standard window creation on the main thread.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                rect,
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // SAFETY: the app keeps its windows (see `App::windows`), so AppKit
        // must not release them on close.
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str(&self.title));
        if !self.toolbar.is_empty() {
            let delegate = chrome::install_toolbar(&window, self.toolbar);
            WINDOW_EXTRAS.with_borrow_mut(|e| e.push((window.clone(), delegate)));
        }
        let (delegate, undo) = undo::install(mtm, &window);
        WINDOW_EXTRAS.with_borrow_mut(|e| e.push((window.clone(), delegate)));
        // Tab follows the views' positions (top-left to bottom-right).
        window.setAutorecalculatesKeyViewLoop(true);
        if let Some(build) = self.content {
            window.setContentView(Some(&build()));
        }
        if let Some(view) = self.view {
            // SAFETY: NSView's designated initialiser.
            let root: Retained<RootView> = unsafe { msg_send![RootView::alloc(mtm), initWithFrame: rect] };
            let node = undo::building(&undo, || batch(|| view.build(&mut Cx::new(Retained::into_super(root.clone())))));
            ROOTS.with_borrow_mut(|r| r.push((root.clone(), node)));
            window.setContentView(Some(&root));
            relayout_all();
        }
        window.center();
        if let Some(name) = &self.autosave {
            let name = NSString::from_str(name);
            // Restore first: setting the name alone doesn't move the window.
            window.setFrameUsingName(&name);
            window.setFrameAutosaveName(&name);
        }
        window
    }
}

/// What the app hears about its environment.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum AppEvent {
    /// Dark Mode or the accent colour changed. Read [`is_dark`] /
    /// [`system_colors`].
    AppearanceChanged,
}

type Handler = Rc<dyn Fn(AppEvent)>;

define_class!(
    // A window's content view: flipped (top-left origin), lays its node out
    // whenever its size changes (same transaction as the window resize).
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AppKitUiRootView"]
    struct RootView;

    impl RootView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            let _: () = unsafe { msg_send![super(self), setFrameSize: size] };
            relayout(self);
        }
    }
);

thread_local! {
    static ROOTS: RefCell<Vec<(Retained<RootView>, Node)>> = const { RefCell::new(Vec::new()) };
    static IN_LAYOUT: Cell<bool> = const { Cell::new(false) };
    /// Objects windows need for the app's lifetime (toolbar delegates).
    static EXTRAS: RefCell<Vec<Retained<NSObject>>> = const { RefCell::new(Vec::new()) };
    /// Objects a window needs while it's open (its delegate, its toolbar's
    /// delegate); released when it closes.
    static WINDOW_EXTRAS: RefCell<Vec<(Retained<NSWindow>, Retained<NSObject>)>> = const { RefCell::new(Vec::new()) };
    static LAYOUT_US: RefCell<Vec<f64>> = const { RefCell::new(Vec::new()) };
    static LAYOUT_PASSES: Cell<u64> = const { Cell::new(0) };
}

fn relayout(root: &RootView) {
    if IN_LAYOUT.with(|l| l.replace(true)) {
        return;
    }
    let t = std::time::Instant::now();
    ROOTS.with_borrow(|roots| {
        for (view, node) in roots {
            if std::ptr::eq(&**view, root) {
                let b = view.bounds().size;
                let scale = view.window().map_or(2.0, |w| w.backingScaleFactor());
                layout::place(node, 0.0, 0.0, b.width, b.height, &layout::Snap(scale));
            }
        }
    });
    LAYOUT_US.with_borrow_mut(|v| {
        // Keep the recent passes only: an app resized for hours mustn't grow.
        if v.len() >= 2 * LAYOUT_KEPT {
            v.drain(..LAYOUT_KEPT);
        }
        v.push(t.elapsed().as_secs_f64() * 1e6);
    });
    LAYOUT_PASSES.with(|n| n.set(n.get() + 1));
    IN_LAYOUT.with(|l| l.set(false));
}

/// Tears down the view tree of `window`'s content (its node is dropped:
/// effects stop, native views go).
pub(crate) fn drop_root(window: &NSWindow) {
    let Some(content) = window.contentView() else { return };
    let node = ROOTS.with_borrow_mut(|r| {
        let i = r.iter().position(|(v, _)| std::ptr::eq(&***v as *const NSView, &*content as *const NSView))?;
        Some(r.remove(i))
    });
    drop(node);
}

/// Whether a layout pass is running.
pub(crate) fn in_layout() -> bool {
    IN_LAYOUT.with(Cell::get)
}

/// Lays every window out now (after AppKit resized part of one).
pub(crate) fn relayout_now() {
    relayout_all();
}

fn relayout_all() {
    view::LAYOUT_DIRTY.with(|d| d.set(false));
    let roots: Vec<Retained<RootView>> = ROOTS.with_borrow(|r| r.iter().map(|(v, _)| v.clone()).collect());
    for r in roots {
        relayout(&r);
    }
}

/// How many layout passes [`layout_times_us`] keeps at least.
const LAYOUT_KEPT: usize = 10_000;

/// Microseconds of the recent layout passes, oldest first (diagnostic; at
/// least the last 10,000).
pub fn layout_times_us() -> Vec<f64> {
    LAYOUT_US.with_borrow(Clone::clone)
}

/// Layout passes so far. With [`layout_times_us`], the times of the passes
/// since an earlier count: `let t = layout_times_us(); &t[t.len() - (layout_passes() - n0) as usize..]`.
pub fn layout_passes() -> u64 {
    LAYOUT_PASSES.with(Cell::get)
}

thread_local! {
    static HANDLERS: RefCell<Vec<Handler>> = const { RefCell::new(Vec::new()) };
    static WINDOWS: RefCell<Vec<Retained<NSWindow>>> = const { RefCell::new(Vec::new()) };
}

/// The application: a name, its windows and the AppKit run loop.
pub struct App {
    name: String,
    windows: Vec<Window>,
    on_launch: Option<Box<dyn FnOnce()>>,
    menus: Vec<(String, Vec<MenuItem>)>,
    settings: Option<Window>,
    documents: Option<Box<dyn FnOnce()>>,
}

impl App {
    /// An app called `name` (shown in the menu bar).
    pub fn new(name: &str) -> Self {
        App { name: name.into(), windows: Vec::new(), on_launch: None, menus: Vec::new(), settings: None, documents: None }
    }

    /// Opens `window` at launch. Call more than once for several windows.
    pub fn window(mut self, window: Window) -> Self {
        self.windows.push(window);
        self
    }

    /// Adds a menu to the menu bar, between Edit and Window ("View", "Go"…).
    /// A menu titled "File" fills the standard File menu instead, above its
    /// Close item. Items run in a [`batch`]; their enabled and checked
    /// states are evaluated when the menu opens or a key equivalent fires.
    pub fn menu(mut self, title: &str, items: Vec<MenuItem>) -> Self {
        self.menus.push((title.into(), items));
        self
    }

    /// The app's Settings window: "Settings…" (⌘,) in the app menu opens it
    /// (built on first use, then brought to the front). It isn't resizable
    /// or listed in the Window menu, and it remembers its position.
    pub fn settings(mut self, window: Window) -> Self {
        self.settings = Some(window);
        self
    }

    /// Makes this a document app for `kind` (see [`DocumentType`]): the File
    /// menu gets New, Open…, Open Recent, Save, Save As… and Revert to
    /// Saved; closing an edited document asks to save it, and quitting
    /// reviews the unsaved ones. With no windows given, the app starts with
    /// an untitled document.
    pub fn documents<T: Clone + PartialEq + 'static>(mut self, kind: DocumentType<T>) -> Self {
        self.documents = Some(Box::new(move || documents::install(kind)));
        self
    }

    /// Runs `f` on the first run-loop pass after the windows are on screen.
    pub fn on_launch(mut self, f: impl FnOnce() + 'static) -> Self {
        self.on_launch = Some(Box::new(f));
        self
    }

    /// Calls `f` for every [`AppEvent`].
    pub fn on_event(self, f: impl Fn(AppEvent) + 'static) -> Self {
        HANDLERS.with_borrow_mut(|h| h.push(Rc::new(f)));
        self
    }

    /// Creates the menu bar and windows, then runs the AppKit run loop. Does
    /// not return while the app is running.
    pub fn run(self) {
        let mtm = mtm();
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        let settings = self.settings.map(|w| {
            SETTINGS.with_borrow_mut(|s| *s = Some(Err(w)));
            Retained::into_super(appkit_views::ActionTarget::new(mtm, || {
                open_settings();
            }))
        });
        let is_documents = self.documents.is_some();
        let mut menus = self.menus;
        let mut file_after = Vec::new();
        if let Some(install) = self.documents {
            install();
            let (above, after) = documents::file_items();
            file_after = after;
            match menus.iter_mut().find(|(t, _)| t == "File") {
                Some((_, items)) => {
                    let own = std::mem::take(items);
                    *items = above.into_iter().chain(own).collect();
                }
                None => menus.insert(0, ("File".into(), above)),
            }
        }
        let start_untitled = is_documents && self.windows.is_empty();
        let targets = menu::install(mtm, &self.name, menus, settings, file_after);
        EXTRAS.with_borrow_mut(|e| e.extend(targets));
        after_batch(|| {
            if view::LAYOUT_DIRTY.with(Cell::get) {
                relayout_all();
            }
        });

        appkit_host::observe_appearance();
        appkit_host::set_event_sink(|event| match event {
            appkit_host::Event::AppearanceChanged => emit(AppEvent::AppearanceChanged),
            // A model queued events: deliver them on the next pass (they
            // arrive inside AppKit callbacks).
            appkit_host::Event::Redraw => later(models::drain_all),
            _ => {}
        });

        // Startup runs before the run loop exists, so it needs its own
        // autorelease pool (see `later`).
        objc2::rc::autoreleasepool(|_| {
            for window in self.windows {
                let w = window.open(mtm);
                w.makeKeyAndOrderFront(None);
                WINDOWS.with_borrow_mut(|ws| ws.push(w));
            }
        });
        if start_untitled {
            objc2::rc::autoreleasepool(|_| documents::new_document());
        }
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
        if let Some(f) = self.on_launch {
            let f = RefCell::new(Some(f));
            later(move || {
                if let Some(f) = f.borrow_mut().take() {
                    f();
                }
            });
        }
        app.run();
    }
}

fn emit(event: AppEvent) {
    let handlers: Vec<Handler> = HANDLERS.with_borrow(|h| h.clone());
    for h in handlers {
        h(event.clone());
    }
}

/// Opens `window` now (while the app runs) and returns it.
pub fn open(window: Window) -> Retained<NSWindow> {
    let mtm = mtm();
    objc2::rc::autoreleasepool(|_| {
        let w = window.open(mtm);
        w.makeKeyAndOrderFront(None);
        WINDOWS.with_borrow_mut(|ws| ws.push(w.clone()));
        w
    })
}

thread_local! {
    /// The Settings window: not yet built, or open.
    static SETTINGS: RefCell<Option<Result<Retained<NSWindow>, Window>>> = const { RefCell::new(None) };
}

/// Opens the app's Settings window (see [`App::settings`]), or brings it to
/// the front, and returns it. `None` if the app has none.
pub fn open_settings() -> Option<Retained<NSWindow>> {
    let mtm = mtm();
    let pending = SETTINGS.with_borrow_mut(|s| match s.take() {
        Some(Err(w)) => Some(w),
        other => {
            *s = other;
            None
        }
    });
    if let Some(mut w) = pending {
        w.fixed = true;
        if w.autosave.is_none() {
            w.autosave = Some("appkit-ui settings".into());
        }
        let window = objc2::rc::autoreleasepool(|_| w.open(mtm));
        window.setExcludedFromWindowsMenu(true);
        SETTINGS.with_borrow_mut(|s| *s = Some(Ok(window)));
    }
    let window = SETTINGS.with_borrow(|s| s.as_ref().and_then(|w| w.as_ref().ok().cloned()))?;
    window.makeKeyAndOrderFront(None);
    Some(window)
}

/// Closes `window` and tears down its views (its node is dropped: effects
/// stop, native views go). The app no longer lists it in [`windows`].
///
/// The close button and ⌘W do the same (except for the Settings window,
/// which is kept and reused).
pub fn close_window(window: &NSWindow) {
    window.close();
    teardown(window);
}

/// Called by the window delegate as `window` closes (close button, ⌘W,
/// `close`): tears it down on the next pass, outside AppKit's close
/// sequence and the delegate's own call.
pub(crate) fn window_will_close(window: &NSWindow) {
    if is_settings(window) {
        return;
    }
    let window = objc2::Message::retain(window);
    later(move || {
        // Reopened since (`makeKeyAndOrderFront` on a kept handle): keep it.
        if !window.isVisible() {
            teardown(&window);
        }
    });
}

fn is_settings(window: &NSWindow) -> bool {
    SETTINGS.with_borrow(|s| matches!(s, Some(Ok(w)) if std::ptr::eq(&**w, window)))
}

/// Drops a closed window's undo history, view tree, delegates and entry in
/// [`windows`]. Running it again does nothing.
pub(crate) fn teardown(window: &NSWindow) {
    if let Some(undo) = window.undoManager() {
        undo.removeAllActions();
    }
    drop_root(window);
    documents::forget(window);
    WINDOWS.with_borrow_mut(|ws| ws.retain(|w| !std::ptr::eq(&**w, window)));
    let extras: Vec<Retained<NSObject>> = WINDOW_EXTRAS.with_borrow_mut(|e| {
        let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(e).into_iter().partition(|(w, _)| std::ptr::eq(&**w, window));
        *e = rest;
        mine.into_iter().map(|(_, x)| x).collect()
    });
    if !extras.is_empty() {
        // The delegates are about to go: AppKit mustn't call them again.
        window.setDelegate(None);
        if let Some(toolbar) = window.toolbar() {
            toolbar.setDelegate(None);
        }
    }
    drop(extras);
}

/// The app's windows, in the order they were opened.
pub fn windows() -> Vec<Retained<NSWindow>> {
    WINDOWS.with_borrow(|w| w.clone())
}

/// Runs `f` on a later pass of the main run loop.
pub fn later(f: impl Fn() + 'static) {
    Native::later(Box::new(f));
}

/// The app's appearance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    /// Follow the system setting.
    System,
    /// Always light.
    Light,
    /// Always dark.
    Dark,
}

/// Sets the app's appearance; every native view follows, and an
/// [`AppEvent::AppearanceChanged`] is delivered.
pub fn set_appearance(appearance: Appearance) {
    let name = match appearance {
        Appearance::System => None,
        // SAFETY: AppKit's appearance name constants.
        Appearance::Light => Some(unsafe { objc2_app_kit::NSAppearanceNameAqua }),
        Appearance::Dark => Some(unsafe { objc2_app_kit::NSAppearanceNameDarkAqua }),
    };
    let appearance = name.and_then(objc2_app_kit::NSAppearance::appearanceNamed);
    NSApplication::sharedApplication(mtm()).setAppearance(appearance.as_deref());
}

/// Ends the app (as ⌘Q does), on the next pass of the run loop. Quitting
/// can wait for answers (a document app asks about unsaved changes) inside
/// a nested run loop; starting it outside the caller's batch keeps the UI
/// updating meanwhile.
pub fn quit() {
    later(|| NSApplication::sharedApplication(mtm()).terminate(None));
}

type Continuation = Box<dyn FnOnce(Box<dyn Any + Send>)>;

thread_local! {
    static CONTINUATIONS: RefCell<HashMap<u64, Continuation>> = RefCell::new(HashMap::new());
    static NEXT_TASK: Cell<u64> = const { Cell::new(0) };
}

/// Runs `work` on a background thread, then `then` with its result on the
/// main thread, inside a [`batch`]. Call from the main thread.
pub fn spawn<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static, then: impl FnOnce(T) + 'static) {
    let _ = mtm();
    let id = NEXT_TASK.with(|n| n.replace(n.get() + 1));
    CONTINUATIONS.with_borrow_mut(|c| {
        c.insert(
            id,
            Box::new(move |result: Box<dyn Any + Send>| {
                then(*result.downcast::<T>().expect("spawned task result type"));
            }),
        )
    });
    std::thread::spawn(move || {
        let result: Box<dyn Any + Send> = Box::new(work());
        // On the main thread the continuation (a main-thread value) is
        // looked up by id.
        Native::on_main(Box::new(move || {
            if let Some(then) = CONTINUATIONS.with_borrow_mut(|c| c.remove(&id)) {
                batch(|| then(result));
            }
        }));
    });
}

/// The guide's chapters, compiled as doc tests so their examples stay right.
#[cfg(doctest)]
mod guide {
    #[doc = include_str!("../README.md")]
    mod readme {}
    #[doc = include_str!("../guide/01-getting-started.md")]
    mod getting_started {}
    #[doc = include_str!("../guide/02-state.md")]
    mod state {}
    #[doc = include_str!("../guide/03-views-and-layout.md")]
    mod views_and_layout {}
    #[doc = include_str!("../guide/04-controls-and-forms.md")]
    mod controls_and_forms {}
    #[doc = include_str!("../guide/05-tables-outlines-editors.md")]
    mod tables_outlines_editors {}
    #[doc = include_str!("../guide/06-app-shell.md")]
    mod app_shell {}
    #[doc = include_str!("../guide/07-live-data.md")]
    mod live_data {}
    #[doc = include_str!("../guide/08-testing-and-shipping.md")]
    mod testing_and_shipping {}
}
