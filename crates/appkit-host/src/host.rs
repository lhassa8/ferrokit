use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSResponder, NSView};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::geometry::{self, Rect};

/// Identifies a mounted native view. Chosen by the caller; stable for the
/// view's lifetime (views are created once and updated in place).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ViewId(pub u64);

#[derive(Debug)]
pub enum AttachError {
    NoHandle,
    NotAppKit,
}

define_class!(
    // Clipping container around every native view: sized to the visible part
    // of the slot, flipped so the child's offset uses the host's top-left space.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AppKitHostClipView"]
    struct ClipView;

    impl ClipView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

impl ClipView {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        this.setClipsToBounds(true);
        this.setHidden(true);
        this
    }
}

struct Mounted {
    clip: Retained<ClipView>,
    view: Retained<NSView>,
}

struct Host {
    /// The host's content view; its layer is the GPU surface.
    content: Retained<NSView>,
    views: HashMap<ViewId, Mounted>,
    /// Views kept hidden regardless of layout (e.g. under an open host overlay).
    suppressed: HashSet<ViewId>,
}

thread_local! {
    static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
}

pub(crate) fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("appkit-host must be used on the main thread")
}

/// Takes the window's content view. Call once the host window exists.
pub fn attach(window: &impl HasWindowHandle) -> Result<(), AttachError> {
    let _ = mtm();
    let handle = window.window_handle().map_err(|_| AttachError::NoHandle)?.as_raw();
    let RawWindowHandle::AppKit(handle) = handle else { return Err(AttachError::NotAppKit) };
    // SAFETY: an AppKit window handle carries a valid NSView pointer while the
    // window lives; we retain it.
    let content: Retained<NSView> =
        unsafe { Retained::retain(handle.ns_view.as_ptr().cast()) }.ok_or(AttachError::NoHandle)?;
    HOST.with_borrow_mut(|host| *host = Some(Host { content, views: HashMap::new(), suppressed: HashSet::new() }));
    Ok(())
}

pub fn is_attached() -> bool {
    HOST.with_borrow(Option::is_some)
}

/// Device pixels per point for the attached window (1.0 before attaching).
pub fn scale() -> f64 {
    HOST.with_borrow(|host| host.as_ref().and_then(|h| h.content.window()).map_or(1.0, |w| w.backingScaleFactor()))
}

pub fn content_view() -> Option<Retained<NSView>> {
    HOST.with_borrow(|host| host.as_ref().map(|h| h.content.clone()))
}

/// Adds `view` (hidden until first placed). Replaces any view with this id.
pub fn mount(id: ViewId, view: Retained<NSView>) {
    let mtm = mtm();
    HOST.with_borrow_mut(|host| {
        let Some(host) = host else { return };
        let clip = ClipView::new(mtm);
        clip.addSubview(&view);
        host.content.addSubview(&clip);
        if let Some(old) = host.views.insert(id, Mounted { clip, view }) {
            old.clip.removeFromSuperview();
        }
    });
}

pub fn unmount(id: ViewId) {
    HOST.with_borrow_mut(|host| {
        let Some(host) = host else { return };
        host.suppressed.remove(&id);
        if let Some(m) = host.views.remove(&id) {
            m.clip.removeFromSuperview();
        }
    });
}

pub fn hide(id: ViewId) {
    HOST.with_borrow(|host| {
        if let Some(m) = host.as_ref().and_then(|h| h.views.get(&id)) {
            m.clip.setHidden(true);
        }
    });
}

/// Records that a view must stay hidden (or may show again). Takes effect at
/// the view's next `place`, so the change lands in the same frame the host
/// draws (toggling it immediately showed a one-frame flash in the spike).
pub fn set_suppressed(id: ViewId, suppressed: bool) {
    HOST.with_borrow_mut(|host| {
        let Some(host) = host else { return };
        if suppressed {
            host.suppressed.insert(id);
        } else {
            host.suppressed.remove(&id);
        }
    });
}

/// Moves a view to the host's layout. Call while the host draws the frame
/// that shows the slot, so the native change and the GPU frame belong together.
/// `bounds`: where the whole view would be. `visible`: the part that may show.
pub fn place(id: ViewId, bounds: Rect, visible: Rect) {
    crate::sync::note_place();
    HOST.with_borrow(|host| {
        let Some(host) = host else { return };
        let Some(m) = host.views.get(&id) else { return };
        if host.suppressed.contains(&id) || visible.is_empty() {
            m.clip.setHidden(true);
            return;
        }
        let scale = host.content.window().map_or(1.0, |w| w.backingScaleFactor());
        let f = geometry::frames(bounds, visible, scale, host.content.bounds().size.height, host.content.isFlipped());
        let ns = |r: Rect| NSRect::new(NSPoint::new(r.x, r.y), NSSize::new(r.width, r.height));
        let (clip, view) = (ns(f.clip), ns(f.view));
        if m.clip.frame() != clip {
            m.clip.setFrame(clip);
        }
        if m.view.frame() != view {
            m.view.setFrame(view);
        }
        if m.clip.isHidden() {
            m.clip.setHidden(false);
        }
    });
}
