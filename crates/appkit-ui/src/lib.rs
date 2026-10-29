//! A native macOS UI framework for Rust: every view is a real AppKit view.
//! Phase 2, unreleased. So far: the app, its windows, the run loop, the
//! default menu bar and appearance tracking (F1.1); signals, batching and
//! background work (F1.2); views, identity and teardown (F1.3); layout with
//! stacks, grids, alignment and subtree-only relayout (F1.4); text fields,
//! secure fields, toggles and checkboxes bound to signals (F1.5); pickers,
//! sliders, steppers, progress, SF Symbols, dividers and scroll views (F1.6);
//! the native table (F2.1); outlines and the source-list sidebar (F2.2).
//!
//! Main thread only.

#![warn(missing_docs)]

mod controls;
mod layout;
mod menu;
mod models;
mod outline;
mod signal;
pub mod testing;
mod view;

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSResponder, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::NSObject;
use objc2_core_foundation::{CFRunLoop, kCFRunLoopCommonModes};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

pub use appkit_host::{is_dark, system_colors};
pub use signal::{Effect, Memo, Signal, after_batch, batch, effect_runs, untrack};
pub use controls::{
    Checkbox, Image, Picker, Progress, Slider, Stepper, TextField, checkbox, image, picker, progress, secure_field, slider,
    stepper, text_field, toggle,
};
pub use layout::layout_stats;
pub use models::{Table, table};
pub use outline::{Outline, TreeNode, outline, sidebar};
pub use appkit_host::table::{Column, ColumnState, Sort, TableModel};
pub use appkit_host::ViewId;
pub use view::{
    Align, AnyView, Axis, Button, Cx, Divider, Dynamic, ForEach, Grid, Identifiable, Label, Modified, Node, Scroll, Size,
    Spacer, Stack, Text, View, ViewExt, Views, button, divider, dynamic, for_each, grid, hstack, label, live_nodes, scroll,
    spacer, text, vstack, watch, Watch,
};

fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("appkit-ui must be used on the main thread")
}

/// A window to open when the app starts.
pub struct Window {
    title: String,
    size: (f64, f64),
    content: Option<Box<dyn FnOnce() -> Retained<NSView>>>,
    view: Option<AnyView>,
}

impl Window {
    /// A titled, resizable window of 800 × 600 points.
    pub fn new(title: &str) -> Self {
        Window { title: title.into(), size: (800.0, 600.0), content: None, view: None }
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

    /// A plain `NSView` as the window's content (escape hatch; no layout).
    pub fn content(mut self, build: impl FnOnce() -> Retained<NSView> + 'static) -> Self {
        self.content = Some(Box::new(build));
        self
    }

    fn open(self, mtm: MainThreadMarker) -> Retained<NSWindow> {
        let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(self.size.0, self.size.1));
        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable;
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
        // Tab follows the views' positions (top-left to bottom-right).
        window.setAutorecalculatesKeyViewLoop(true);
        if let Some(build) = self.content {
            window.setContentView(Some(&build()));
        }
        if let Some(view) = self.view {
            // SAFETY: NSView's designated initialiser.
            let root: Retained<RootView> = unsafe { msg_send![RootView::alloc(mtm), initWithFrame: rect] };
            let node = batch(|| view.build(&mut Cx::new(Retained::into_super(root.clone()))));
            ROOTS.with_borrow_mut(|r| r.push((root.clone(), node)));
            window.setContentView(Some(&root));
            relayout_all();
        }
        window.center();
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
    static LAYOUT_US: RefCell<Vec<f64>> = const { RefCell::new(Vec::new()) };
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
    LAYOUT_US.with_borrow_mut(|v| v.push(t.elapsed().as_secs_f64() * 1e6));
    IN_LAYOUT.with(|l| l.set(false));
}

fn relayout_all() {
    view::LAYOUT_DIRTY.with(|d| d.set(false));
    let roots: Vec<Retained<RootView>> = ROOTS.with_borrow(|r| r.iter().map(|(v, _)| v.clone()).collect());
    for r in roots {
        relayout(&r);
    }
}

/// Microseconds of each layout pass so far (diagnostic).
pub fn layout_times_us() -> Vec<f64> {
    LAYOUT_US.with_borrow(Clone::clone)
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
}

impl App {
    /// An app called `name` (shown in the menu bar).
    pub fn new(name: &str) -> Self {
        App { name: name.into(), windows: Vec::new(), on_launch: None }
    }

    /// Opens `window` at launch. Call more than once for several windows.
    pub fn window(mut self, window: Window) -> Self {
        self.windows.push(window);
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
        menu::install(mtm, &self.name);
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

/// The app's windows, in the order they were opened.
pub fn windows() -> Vec<Retained<NSWindow>> {
    WINDOWS.with_borrow(|w| w.clone())
}

/// Runs `f` on a later pass of the main run loop.
pub fn later(f: impl Fn() + 'static) {
    let main = CFRunLoop::main().expect("main run loop");
    // Blocks run outside NSApplication's per-event autorelease pool, so give
    // each its own: without it, everything AppKit autoreleases inside (e.g.
    // new controls) is never released.
    let block = RcBlock::new(move || objc2::rc::autoreleasepool(|_| f()));
    // SAFETY: runs on the main run loop; touches main-thread state only.
    unsafe { main.perform_block(kCFRunLoopCommonModes.map(|m| m.as_ref()), Some(&block)) };
    main.wake_up();
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

/// Ends the app (as ⌘Q does).
pub fn quit() {
    NSApplication::sharedApplication(mtm()).terminate(None);
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
        let result: Arc<Mutex<Option<Box<dyn Any + Send>>>> = Arc::new(Mutex::new(Some(Box::new(work()))));
        let main = CFRunLoop::main().expect("main run loop");
        let block = RcBlock::new(move || {
            let result = result.lock().expect("task result").take();
            let then = CONTINUATIONS.with_borrow_mut(|c| c.remove(&id));
            if let (Some(result), Some(then)) = (result, then) {
                // Own pool: see `later`.
                objc2::rc::autoreleasepool(|_| batch(|| then(result)));
            }
        });
        // SAFETY: CFRunLoopPerformBlock and CFRunLoopWakeUp may be called from
        // any thread; the block runs on the main run loop, where the
        // continuation (a main-thread value) is looked up by id.
        unsafe { main.perform_block(kCFRunLoopCommonModes.map(|m| m.as_ref()), Some(&block)) };
        main.wake_up();
    });
}
