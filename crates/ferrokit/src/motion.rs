//! Motion: changes made inside [`animated`] move, fade and recolour instead
//! of jumping, and views with a [`Transition`] fade or slide in and out.
//!
//! An animated change runs as a normal batch, with the animation pending.
//! The layout pass after it, the property changes that effects queued
//! ([`apply`]), the entrances and the exits all run inside one
//! `NSAnimationContext` group with implicit animation allowed, so AppKit
//! animates every frame, opacity and layer colour that changed. Views that
//! were never placed take their first frame without animating. With Reduce
//! Motion on, nothing moves: layout lands at once, and slides become fades.

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2_app_kit::{NSAnimationContext, NSView, NSWorkspace};
use objc2_foundation::NSRect;

use crate::Edge;
use crate::signal::batch;

/// How an animated change moves (see [`animated_with`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Animation {
    duration: f64,
    curve: Curve,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Curve {
    EaseInOut,
    EaseOut,
    EaseIn,
    Linear,
}

impl Default for Animation {
    /// A quarter of a second, easing in and out: the system's own pace.
    fn default() -> Self {
        Animation { duration: 0.25, curve: Curve::EaseInOut }
    }
}

impl Animation {
    /// Easing in and out over `seconds`: for things that move.
    pub fn ease_in_out(seconds: f64) -> Animation {
        Animation { duration: seconds, curve: Curve::EaseInOut }
    }

    /// Starting fast and settling over `seconds`: for things that appear.
    pub fn ease_out(seconds: f64) -> Animation {
        Animation { duration: seconds, curve: Curve::EaseOut }
    }

    /// Starting slowly over `seconds`: for things that leave.
    pub fn ease_in(seconds: f64) -> Animation {
        Animation { duration: seconds, curve: Curve::EaseIn }
    }

    /// At a constant pace over `seconds`.
    pub fn linear(seconds: f64) -> Animation {
        Animation { duration: seconds, curve: Curve::Linear }
    }

    /// How long it takes, in seconds.
    pub fn duration(&self) -> f64 {
        self.duration
    }
}

/// How a view comes and goes when it's added or removed inside
/// [`animated`] (see [`ViewExt::transition`](crate::ViewExt::transition)).
/// Outside an animation it appears and disappears at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Transition {
    /// Fades in and out.
    Fade,
    /// Fades while sliding a short way in from the edge, and back out to
    /// it. With Reduce Motion on, it only fades.
    Slide(Edge),
}

/// How far a sliding view travels, in points.
const SLIDE: f64 = 24.0;

/// A view coming in: its views, how, and the opacity it ends at.
struct Entrance {
    views: Vec<Retained<NSView>>,
    transition: Transition,
    alpha: f64,
}

thread_local! {
    /// The animation of the change in progress (set by `animated`, taken
    /// by the layout hook).
    static PENDING: Cell<Option<Animation>> = const { Cell::new(None) };
    /// Inside the animation group: frames set now animate.
    static RUNNING: Cell<bool> = const { Cell::new(false) };
    static QUEUED: RefCell<Vec<Box<dyn FnOnce()>>> = RefCell::new(Vec::new());
    static ENTERING: RefCell<Vec<Entrance>> = const { RefCell::new(Vec::new()) };
    static EXITING: RefCell<Vec<(Vec<Retained<NSView>>, Transition)>> = const { RefCell::new(Vec::new()) };
    /// Views on their way out (removed when their animation ends).
    static LEAVING: Cell<usize> = const { Cell::new(0) };
    static REDUCE_MOTION: Cell<Option<bool>> = const { Cell::new(None) };
}

/// Runs `f` (as a [`batch`]) and animates what it changes: views that move
/// or resize because of it, opacity and colours set by
/// [`opacity_when`](crate::ViewExt::opacity_when) and
/// [`background_when`](crate::ViewExt::background_when), and views with a
/// [`transition`](crate::ViewExt::transition) that come or go. Over a
/// quarter of a second, easing in and out.
///
/// ```no_run
/// use ferrokit::*;
///
/// let expanded = Signal::new(false);
/// let more = button("More", move || animated(|| expanded.set(!expanded.get())));
/// let details = dynamic(move || {
///     if expanded.get() { AnyView::new(text("Details").transition(Transition::Fade)) } else { AnyView::new(spacer()) }
/// });
/// # let _ = (more, details);
/// ```
pub fn animated<R>(f: impl FnOnce() -> R) -> R {
    animated_with(Animation::default(), f)
}

/// [`animated`] with your own timing.
pub fn animated_with<R>(animation: Animation, f: impl FnOnce() -> R) -> R {
    PENDING.with(|p| p.set(Some(animation)));
    batch(f)
}

/// Whether the user asked for less motion (System Settings › Accessibility
/// › Display › Reduce motion). Animated changes respect it on their own.
pub fn reduce_motion() -> bool {
    REDUCE_MOTION.with(Cell::get).unwrap_or_else(|| NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion())
}

/// Overrides [`reduce_motion`] (`None`: the system's setting), for tests.
pub(crate) fn set_reduce_motion(on: Option<bool>) {
    REDUCE_MOTION.with(|r| r.set(on));
}

/// Whether an animated change is in progress (pending or running).
pub(crate) fn animating() -> bool {
    // `try_with`: nodes also drop while the thread's state is torn down.
    PENDING.try_with(Cell::get).ok().flatten().is_some() || RUNNING.try_with(Cell::get).unwrap_or(false)
}

/// Runs `change` inside the animation if one is pending (after layout),
/// else now.
pub(crate) fn apply(change: impl FnOnce() + 'static) {
    if PENDING.with(Cell::get).is_some() {
        QUEUED.with_borrow_mut(|q| q.push(Box::new(change)));
    } else {
        change();
    }
}

/// `views` are new and come in with `transition`, ending at `alpha`.
pub(crate) fn enter(views: Vec<Retained<NSView>>, transition: Transition, alpha: f64) {
    for v in &views {
        // A layer now, so the fade has a starting value to animate from.
        v.setWantsLayer(true);
        v.setAlphaValue(0.0);
    }
    ENTERING.with_borrow_mut(|e| e.push(Entrance { views, transition, alpha }));
}

/// `views` leave with `transition`: they stay until it ends, then go.
pub(crate) fn exit(views: Vec<Retained<NSView>>, transition: Transition) {
    EXITING.with_borrow_mut(|e| e.push((views, transition)));
}

/// How many views are still animating out (diagnostic, for checks).
pub(crate) fn leaving() -> usize {
    LEAVING.with(Cell::get)
}

/// Sets `view`'s frame: animated inside an animated change, except for a
/// view never placed (it starts where it belongs).
pub(crate) fn set_frame(view: &NSView, frame: NSRect) {
    let unplaced = view.frame().size.width == 0.0 && view.frame().size.height == 0.0;
    if RUNNING.with(Cell::get) && unplaced {
        without_animation(|| view.setFrame(frame));
    } else {
        view.setFrame(frame);
    }
}

fn without_animation(f: impl FnOnce()) {
    NSAnimationContext::beginGrouping();
    let cx = NSAnimationContext::currentContext();
    cx.setDuration(0.0);
    cx.setAllowsImplicitAnimation(false);
    f();
    NSAnimationContext::endGrouping();
}

/// The layout hook: runs `layout` (and everything an animated change
/// queued) inside the animation if one is pending.
pub(crate) fn run_layout(layout: impl FnOnce()) {
    let Some(animation) = PENDING.with(|p| p.take()) else {
        layout();
        return;
    };
    let reduce = reduce_motion();
    let mut layout = Some(layout);
    if reduce {
        // Nothing moves: the layout lands at once; only fades animate.
        if let Some(l) = layout.take() {
            l();
        }
    }
    NSAnimationContext::beginGrouping();
    let cx = NSAnimationContext::currentContext();
    cx.setDuration(animation.duration);
    cx.setAllowsImplicitAnimation(true);
    // SAFETY: Core Animation's own timing function names.
    let name = unsafe {
        use objc2_quartz_core::*;
        match animation.curve {
            Curve::EaseInOut => kCAMediaTimingFunctionEaseInEaseOut,
            Curve::EaseOut => kCAMediaTimingFunctionEaseOut,
            Curve::EaseIn => kCAMediaTimingFunctionEaseIn,
            Curve::Linear => kCAMediaTimingFunctionLinear,
        }
    };
    cx.setTimingFunction(Some(&objc2_quartz_core::CAMediaTimingFunction::functionWithName(name)));
    RUNNING.with(|r| r.set(true));
    if let Some(l) = layout.take() {
        l();
    }
    for change in QUEUED.with_borrow_mut(std::mem::take) {
        change();
    }
    for e in ENTERING.with_borrow_mut(std::mem::take) {
        // Slides start off by a little toward their edge.
        if let (Transition::Slide(edge), false) = (e.transition, reduce) {
            for v in &e.views {
                let end = v.frame();
                without_animation(|| v.setFrame(offset(end, edge)));
                v.setFrame(end);
            }
        }
        for v in &e.views {
            v.setAlphaValue(e.alpha);
        }
    }
    let exits = EXITING.with_borrow_mut(std::mem::take);
    if !exits.is_empty() {
        // The handler goes on before the changes: the group waits only for
        // animations added after it. When they end, the views go.
        let gone: Vec<Retained<NSView>> = exits.iter().flat_map(|(v, _)| v.iter().cloned()).collect();
        LEAVING.with(|l| l.set(l.get() + gone.len()));
        let gone = RefCell::new(Some(gone));
        let done = block2::RcBlock::new(move || {
            if let Some(views) = gone.borrow_mut().take() {
                LEAVING.with(|l| l.set(l.get().saturating_sub(views.len())));
                for v in views {
                    v.removeFromSuperview();
                }
            }
        });
        cx.setCompletionHandler(Some(&done));
    }
    for (views, transition) in exits {
        for v in &views {
            if let (Transition::Slide(edge), false) = (transition, reduce) {
                v.setFrame(offset(v.frame(), edge));
            }
            v.setAlphaValue(0.0);
        }
    }
    RUNNING.with(|r| r.set(false));
    NSAnimationContext::endGrouping();
}

/// `frame` moved toward `edge` (the views' containers are flipped: down is
/// +y).
fn offset(mut frame: NSRect, edge: Edge) -> NSRect {
    match edge {
        Edge::Above => frame.origin.y -= SLIDE,
        Edge::Below => frame.origin.y += SLIDE,
        Edge::Leading => frame.origin.x -= SLIDE,
        Edge::Trailing => frame.origin.x += SLIDE,
    }
    frame
}
