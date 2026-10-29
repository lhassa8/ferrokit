//! The platform seam.
//!
//! The framework's core reaches the native toolkit only through
//! [`Platform`]: the signals ([`crate::signal`]), the node tree and its
//! identity, the layout pass ([`crate::layout`]) and scheduling ([`crate::later`],
//! [`crate::spawn`], [`crate::batch`]). [`AppKit`] implements it, and
//! [`Native`] names the implementation in use.
//!
//! Everything else is a platform's own: its controls, menus, toolbar,
//! dialogs, tables and outlines, undo, stored settings, frame callbacks and
//! test helpers (the AppKit ones are `controls`, `chrome`, `menu`,
//! `dialogs`, `models`, `outline`, `undo`, `store`, `frame`, `testing`). A
//! second platform (Windows is planned, not started) would implement
//! `Platform` and provide those, keeping the core and the public view API.

use objc2_app_kit::{NSScrollView, NSSplitView, NSView};
use objc2_core_foundation::{CFRunLoop, kCFRunLoopCommonModes};
use objc2_foundation::{NSPoint, NSRect, NSSize};

use crate::view::Size;

/// A rectangle in a parent view's coordinates (points; origin top-left, as
/// the root view is flipped).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// What the core needs from a native toolkit.
pub(crate) trait Platform {
    /// A native view.
    type View: ?Sized;
    /// A scrolling container (its document view is a `View`).
    type Scroll: ?Sized;
    /// A split container of panes.
    type Split: ?Sized;

    /// A control's natural size.
    fn fitting_size(view: &Self::View) -> Size;
    /// Places `view` (does nothing if it's already there).
    fn place(view: &Self::View, rect: Rect);
    /// Sizes a view without moving it (a scroll view's document).
    fn set_size(view: &Self::View, size: Size);
    /// Places a scroll view; returns the size of its visible area.
    fn place_scroll(scroll: &Self::Scroll, rect: Rect) -> Size;
    /// Places a split view and re-tiles its panes.
    fn place_split(split: &Self::Split, rect: Rect);
    /// Moves a split view's first divider.
    fn set_divider(split: &Self::Split, position: f64);
    /// A view's own size (a split pane, after tiling).
    fn size(view: &Self::View) -> Size;

    /// Runs `f` on a later pass of the main run loop. Main thread only.
    fn later(f: Box<dyn Fn()>);
    /// Runs `f` on the main thread, from any thread.
    fn on_main(f: Box<dyn FnOnce() + Send>);
    /// Runs `f` and frees the toolkit's temporary objects it made (an
    /// autorelease pool on AppKit).
    fn scope<R>(f: impl FnOnce() -> R) -> R;
}

/// The platform in use.
pub(crate) type Native = AppKit;

/// macOS: AppKit, through objc2.
pub(crate) struct AppKit;

fn ns(r: Rect) -> NSRect {
    NSRect::new(NSPoint::new(r.x, r.y), NSSize::new(r.w, r.h))
}

impl Platform for AppKit {
    type View = NSView;
    type Scroll = NSScrollView;
    type Split = NSSplitView;

    fn fitting_size(view: &NSView) -> Size {
        let f = view.fittingSize();
        Size { w: f.width, h: f.height }
    }

    fn place(view: &NSView, rect: Rect) {
        let r = ns(rect);
        if view.frame() != r {
            view.setFrame(r);
        }
    }

    fn set_size(view: &NSView, size: Size) {
        let s = NSSize::new(size.w, size.h);
        if view.frame().size != s {
            view.setFrameSize(s);
        }
    }

    fn place_scroll(scroll: &NSScrollView, rect: Rect) -> Size {
        let r = ns(rect);
        if scroll.frame() != r {
            scroll.setFrame(r);
        }
        let v = scroll.contentSize();
        Size { w: v.width, h: v.height }
    }

    fn place_split(split: &NSSplitView, rect: Rect) {
        let r = ns(rect);
        if split.frame() != r {
            split.setFrame(r);
            split.adjustSubviews();
        }
    }

    fn set_divider(split: &NSSplitView, position: f64) {
        split.setPosition_ofDividerAtIndex(position, 0);
    }

    fn size(view: &NSView) -> Size {
        let b = view.bounds().size;
        Size { w: b.width, h: b.height }
    }

    fn later(f: Box<dyn Fn()>) {
        let main = CFRunLoop::main().expect("main run loop");
        // Blocks run outside NSApplication's per-event autorelease pool, so
        // give each its own: without it, everything AppKit autoreleases
        // inside (e.g. new controls) is never released.
        let block = block2::RcBlock::new(move || Self::scope(&f));
        // SAFETY: runs on the main run loop; touches main-thread state only.
        unsafe { main.perform_block(kCFRunLoopCommonModes.map(|m| m.as_ref()), Some(&block)) };
        main.wake_up();
    }

    fn on_main(f: Box<dyn FnOnce() + Send>) {
        let main = CFRunLoop::main().expect("main run loop");
        let f = std::sync::Mutex::new(Some(f));
        let block = block2::RcBlock::new(move || {
            if let Some(f) = f.lock().expect("main-thread task").take() {
                Self::scope(f);
            }
        });
        // SAFETY: CFRunLoopPerformBlock and CFRunLoopWakeUp may be called
        // from any thread; the block runs on the main run loop.
        unsafe { main.perform_block(kCFRunLoopCommonModes.map(|m| m.as_ref()), Some(&block)) };
        main.wake_up();
    }

    fn scope<R>(f: impl FnOnce() -> R) -> R {
        objc2::rc::autoreleasepool(|_| f())
    }
}
