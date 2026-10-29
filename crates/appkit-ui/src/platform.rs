//! The platform seam.
//!
//! The framework's core reaches the native toolkit only through this
//! module: [`Platform`]'s operations and the handle names below
//! (`NativeView`, `NativeScroll`, `NativeSplit`, `NativeObject`,
//! `MainThread`). The core is the signals (`signal`), the node tree with
//! its identity, teardown and modifiers (`view`), the layout pass
//! (`layout`) and scheduling (`later`, `spawn`, `batch`); none of those
//! files names an AppKit or objc2 type. [`AppKit`] implements the trait,
//! and [`Native`] names the implementation in use.
//!
//! Everything else is a platform's own: its views and controls
//! (`appkit_views`, `controls`), menus, toolbar and split view (`chrome`,
//! `menu`), dialogs, documents, tables, grids and outlines (`models`,
//! `collection`, `outline`), undo, stored settings, frame callbacks, the
//! app shell (`App`, `Window` in `lib`) and test helpers. A second platform
//! (Windows is planned, not started) would implement `Platform`, define the
//! handle names, and provide those, keeping the core and the public view
//! API.

use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSScrollView, NSSplitView, NSView};
use objc2_core_foundation::{CFRunLoop, kCFRunLoopCommonModes};
use objc2_foundation::{NSObject, NSPoint, NSRect, NSSize, NSString};

use crate::view::Size;

// The handles the core keeps, named for the platform in use: the node tree,
// `Cx` and the modifiers use these names, never AppKit's. A second platform
// would define its own (behind `cfg`).

/// A native view the core holds (a leaf, a container).
pub(crate) type NativeView = Retained<NSView>;
/// A native scrolling container.
pub(crate) type NativeScroll = Retained<NSScrollView>;
/// A native split container.
pub(crate) type NativeSplit = Retained<NSSplitView>;
/// A native helper object a node keeps alive (targets, delegates).
pub(crate) type NativeObject = Retained<NSObject>;
/// Proof of being on the UI thread.
pub(crate) type MainThread = MainThreadMarker;

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
    /// A scroll view's visible size now, after it re-tiles (its scroll bars
    /// may have appeared or gone since the document was sized).
    fn visible_size(scroll: &Self::Scroll) -> Size;
    /// Places a split view and re-tiles its panes.
    fn place_split(split: &Self::Split, rect: Rect);
    /// Moves a split view's first divider.
    fn set_divider(split: &Self::Split, position: f64);
    /// A view's own size (a split pane, after tiling).
    fn size(view: &Self::View) -> Size;

    /// Adds `view` to `container`.
    fn add_subview(container: &Self::View, view: &Self::View);
    /// Takes `view` out of its container (teardown).
    fn remove(view: &Self::View);
    /// The scroll container as a plain view.
    fn scroll_as_view(scroll: &NativeScroll) -> NativeView;
    /// The split container as a plain view.
    fn split_as_view(split: &NativeSplit) -> NativeView;
    /// Names a view for assistive technology (VoiceOver), showing it to it.
    fn set_accessibility_label(view: &Self::View, label: &str);
    /// Shows a view to assistive technology, or hides it.
    fn set_accessibility_element(view: &Self::View, on: bool);
    /// Gives `views` a right-click menu; returns the objects to keep alive.
    fn set_context_menu(mtm: MainThread, views: &[NativeView], items: Vec<crate::chrome::MenuItem>) -> Vec<NativeObject>;

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

    fn visible_size(scroll: &NSScrollView) -> Size {
        scroll.tile();
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

    fn add_subview(container: &NSView, view: &NSView) {
        container.addSubview(view);
    }

    fn remove(view: &NSView) {
        view.removeFromSuperview();
    }

    fn scroll_as_view(scroll: &NativeScroll) -> NativeView {
        Retained::into_super(scroll.clone())
    }

    fn split_as_view(split: &NativeSplit) -> NativeView {
        Retained::into_super(split.clone())
    }

    fn set_accessibility_label(view: &NSView, label: &str) {
        // A scrolled view (a text editor, a table): VoiceOver reads the document.
        if let Some(doc) = view.downcast_ref::<NSScrollView>().and_then(|s| s.documentView()) {
            Self::set_accessibility_label(&doc, label);
        }
        let label = NSString::from_str(label);
        Self::set_accessibility_element(view, true);
        // Cell-based controls (text fields, sliders, pop-ups) are exposed
        // through their cell on macOS 27, so the cell gets the label too.
        // SAFETY: NSAccessibility setters on main-thread objects; `cell` is
        // only sent to views that answer it.
        unsafe {
            let () = objc2::msg_send![view, setAccessibilityLabel: &*label];
            if objc2::runtime::NSObjectProtocol::respondsToSelector(view, objc2::sel!(cell)) {
                let cell: Option<Retained<objc2::runtime::AnyObject>> = objc2::msg_send![view, cell];
                if let Some(cell) = cell {
                    let () = objc2::msg_send![&*cell, setAccessibilityLabel: &*label];
                }
            }
        }
    }

    fn set_accessibility_element(view: &NSView, on: bool) {
        // SAFETY: as above.
        unsafe {
            let () = objc2::msg_send![view, setAccessibilityElement: on];
            if objc2::runtime::NSObjectProtocol::respondsToSelector(view, objc2::sel!(cell)) {
                let cell: Option<Retained<objc2::runtime::AnyObject>> = objc2::msg_send![view, cell];
                if let Some(cell) = cell {
                    let () = objc2::msg_send![&*cell, setAccessibilityElement: on];
                }
            }
        }
    }

    fn set_context_menu(mtm: MainThread, views: &[NativeView], items: Vec<crate::chrome::MenuItem>) -> Vec<NativeObject> {
        let (menu, targets) = crate::chrome::build_menu(mtm, items);
        for v in views {
            // SAFETY: the caller's node keeps the menu's targets alive.
            unsafe { crate::chrome::menu_host(v).setMenu(Some(&menu)) };
        }
        targets
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
