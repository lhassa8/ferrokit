use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSResponder, NSScrollView, NSView};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::geometry::{self, Rect};

/// Identifies a mounted native view. Chosen by the caller; stable for the
/// view's lifetime (views are created once and updated in place).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ViewId(pub u64);

#[derive(Debug)]
/// Why [`attach`] failed.
pub enum AttachError {
    /// The window has no handle (yet).
    NoHandle,
    /// The window isn't an AppKit window.
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

pub(crate) struct Mounted {
    clip: Retained<ClipView>,
    pub(crate) view: Retained<NSView>,
    /// Visible part in host coordinates, as of the last `place`.
    pub(crate) visible: Option<Rect>,
    /// Frame number of the last `place`.
    placed_frame: u64,
}

pub(crate) struct Host {
    /// The host's content view; its layer is the GPU surface.
    pub(crate) content: Retained<NSView>,
    pub(crate) views: HashMap<ViewId, Mounted>,
    /// Views kept hidden regardless of layout (e.g. under an open host overlay).
    suppressed: HashSet<ViewId>,
    frame: u64,
}

thread_local! {
    static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
}

pub(crate) fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("appkit-host must be used on the main thread")
}

impl Mounted {
    pub(crate) fn clip_frame(&self) -> NSRect {
        self.clip.frame()
    }

    pub(crate) fn shown(&self) -> bool {
        self.visible.is_some() && !self.clip.isHidden()
    }
}

pub(crate) fn with_host<R>(f: impl FnOnce(&Host) -> R) -> Option<R> {
    HOST.with_borrow(|host| host.as_ref().map(f))
}

fn with_host_mut<R>(f: impl FnOnce(&mut Host) -> R) -> Option<R> {
    HOST.with_borrow_mut(|host| host.as_mut().map(f))
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
    HOST.with_borrow_mut(|host| {
        *host = Some(Host { content, views: HashMap::new(), suppressed: HashSet::new(), frame: 0 })
    });
    crate::keys::install();
    crate::appearance::observe_appearance();
    Ok(())
}

/// Whether [`attach`] has succeeded.
pub fn is_attached() -> bool {
    HOST.with_borrow(Option::is_some)
}

/// Device pixels per point for the attached window (1.0 before attaching).
pub fn scale() -> f64 {
    with_host(|h| h.content.window().map(|w| w.backingScaleFactor())).flatten().unwrap_or(1.0)
}

/// The host's content view (whose layer is the GPU surface).
pub fn content_view() -> Option<Retained<NSView>> {
    with_host(|h| h.content.clone())
}

/// The mounted view itself (not its clip container).
pub fn view(id: ViewId) -> Option<Retained<NSView>> {
    with_host(|h| h.views.get(&id).map(|m| m.view.clone())).flatten()
}

/// Whether a view with this id is mounted.
pub fn is_mounted(id: ViewId) -> bool {
    with_host(|h| h.views.contains_key(&id)).unwrap_or(false)
}

/// Adds `view` (hidden until first placed). Replaces any view with this id.
pub fn mount(id: ViewId, view: Retained<NSView>) {
    let mtm = mtm();
    with_host_mut(|host| {
        let clip = ClipView::new(mtm);
        clip.addSubview(&view);
        host.content.addSubview(&clip);
        if let Some(old) = host.views.insert(id, Mounted { clip, view, visible: None, placed_frame: 0 }) {
            old.clip.removeFromSuperview();
        }
    });
}

/// Removes a view from the window (the view is released).
pub fn unmount(id: ViewId) {
    with_host_mut(|host| {
        host.suppressed.remove(&id);
        if let Some(m) = host.views.remove(&id) {
            m.clip.removeFromSuperview();
        }
    });
}

/// Hides a view until its next [`place`].
pub fn hide(id: ViewId) {
    with_host(|host| {
        if let Some(m) = host.views.get(&id) {
            m.clip.setHidden(true);
        }
    });
}

/// Records that a view must stay hidden (or may show again). Takes effect at
/// the view's next `place`, so the change lands in the same frame the host
/// draws (toggling it immediately showed a one-frame flash in the spike).
pub fn set_suppressed(id: ViewId, suppressed: bool) {
    with_host_mut(|host| {
        if suppressed {
            host.suppressed.insert(id);
        } else {
            host.suppressed.remove(&id);
        }
    });
}

/// Whether [`set_suppressed`] is keeping the view hidden.
pub fn is_suppressed(id: ViewId) -> bool {
    with_host(|h| h.suppressed.contains(&id)).unwrap_or(false)
}

/// Starts a host frame. Views not placed between this and [`end_frame`] are
/// hidden: they are no longer in the host's layout.
pub fn begin_frame() {
    crate::events::frame_started();
    with_host_mut(|host| host.frame += 1);
}

/// Ends a host frame: hides every view that wasn't placed during it.
pub fn end_frame() {
    with_host(|host| {
        for m in host.views.values() {
            if m.placed_frame != host.frame && !m.clip.isHidden() {
                m.clip.setHidden(true);
            }
        }
    });
    with_host_mut(|host| {
        for m in host.views.values_mut() {
            if m.placed_frame != host.frame {
                m.visible = None;
            }
        }
    });
}

/// Mounted views whose visible part (as last placed) intersects `area`.
pub fn views_in(area: Rect) -> Vec<ViewId> {
    with_host(|h| {
        h.views.iter().filter(|(_, m)| m.visible.is_some_and(|v| v.intersection(&area).is_some())).map(|(id, _)| *id).collect()
    })
    .unwrap_or_default()
}

/// Moves a view to the host's layout. Call while the host draws the frame
/// that shows the slot, so the native change and the GPU frame belong together.
/// `bounds`: where the whole view would be. `visible`: the part that may show.
pub fn place(id: ViewId, bounds: Rect, visible: Rect) {
    crate::sync::note_place();
    with_host_mut(|host| {
        let frame = host.frame;
        let suppressed = host.suppressed.contains(&id);
        let scale = host.content.window().map_or(1.0, |w| w.backingScaleFactor());
        let (height, flipped) = (host.content.bounds().size.height, host.content.isFlipped());
        let Some(m) = host.views.get_mut(&id) else { return };
        m.placed_frame = frame;
        m.visible = (!visible.is_empty()).then_some(visible);
        if suppressed || visible.is_empty() {
            m.clip.setHidden(true);
            return;
        }
        let f = geometry::frames(bounds, visible, scale, height, flipped);
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

/// The view that takes keyboard focus for a mounted view: a scroll view's
/// document view, otherwise the view itself.
pub(crate) fn focus_target(view: &NSView) -> Retained<NSView> {
    use objc2::Message as _;
    match view.downcast_ref::<NSScrollView>() {
        Some(scroll) => scroll.documentView().unwrap_or_else(|| view.retain()),
        None => view.retain(),
    }
}
