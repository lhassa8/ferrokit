//! A native macOS UI framework for Rust: every view is a real AppKit view.
//! Phase 2, unreleased. This is milestone F1.1: the app, its windows, the run
//! loop, the default menu bar and appearance tracking. Views, signals and
//! layout follow (F1.2–F1.4); until then a window's content is a plain
//! `NSView`.
//!
//! Main thread only.

#![warn(missing_docs)]

mod menu;

use std::cell::RefCell;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_core_foundation::{CFRunLoop, kCFRunLoopCommonModes};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

pub use appkit_host::{is_dark, system_colors};

fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("appkit-ui must be used on the main thread")
}

/// A window to open when the app starts.
pub struct Window {
    title: String,
    size: (f64, f64),
    content: Option<Box<dyn FnOnce() -> Retained<NSView>>>,
}

impl Window {
    /// A titled, resizable window of 800 × 600 points.
    pub fn new(title: &str) -> Self {
        Window { title: title.into(), size: (800.0, 600.0), content: None }
    }

    /// The content size in points.
    pub fn size(mut self, width: f64, height: f64) -> Self {
        self.size = (width, height);
        self
    }

    /// The window's content view, built when the window opens.
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
        if let Some(build) = self.content {
            window.setContentView(Some(&build()));
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

        appkit_host::observe_appearance();
        appkit_host::set_event_sink(|event| {
            if event == appkit_host::Event::AppearanceChanged {
                emit(AppEvent::AppearanceChanged);
            }
        });

        for window in self.windows {
            let w = window.open(mtm);
            w.makeKeyAndOrderFront(None);
            WINDOWS.with_borrow_mut(|ws| ws.push(w));
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

/// The app's windows, in the order they were opened.
pub fn windows() -> Vec<Retained<NSWindow>> {
    WINDOWS.with_borrow(|w| w.clone())
}

/// Runs `f` on a later pass of the main run loop.
pub fn later(f: impl Fn() + 'static) {
    let main = CFRunLoop::main().expect("main run loop");
    let block = RcBlock::new(f);
    // SAFETY: runs on the main run loop; touches main-thread state only.
    unsafe { main.perform_block(kCFRunLoopCommonModes.map(|m| m.as_ref()), Some(&block)) };
    main.wake_up();
}

/// Ends the app (as ⌘Q does).
pub fn quit() {
    NSApplication::sharedApplication(mtm()).terminate(None);
}
