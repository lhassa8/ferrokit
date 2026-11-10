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
//!
//! Springs ([`Animation::spring`]) overshoot and settle, which Core
//! Animation's curves can't: frames under a spring are set a frame at a
//! time instead, and a new change takes over from where a view is. The same
//! driver animates values ([`animate`], [`keyframes`]), and matched
//! geometry ([`ViewExt::matched_geometry`](crate::ViewExt::matched_geometry))
//! starts a view where the one it replaces was.

use ferrokit_core::kept::{UiCell, UiLocal};
use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSAnimationContext, NSView, NSWorkspace};
use objc2_foundation::{NSPoint, NSRect, NSSize};

use crate::Edge;
use crate::signal::{Signal, batch};

/// How an animated change moves (see [`animated_with`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Animation {
    duration: f64,
    curve: Curve,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Curve {
    EaseInOut,
    EaseOut,
    EaseIn,
    Linear,
    /// A damped spring: its period undamped, in seconds, and its damping
    /// (1 settles without overshooting; less bounces).
    Spring { response: f64, damping: f64 },
}

impl Curve {
    /// Progress at `t` (0 to 1) along an easing curve: CSS's cubic Béziers,
    /// as Core Animation's named curves are.
    fn eased(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        let (x1, y1, x2, y2) = match self {
            Curve::EaseInOut => (0.42, 0.0, 0.58, 1.0),
            Curve::EaseOut => (0.0, 0.0, 0.58, 1.0),
            Curve::EaseIn => (0.42, 0.0, 1.0, 1.0),
            Curve::Linear | Curve::Spring { .. } => return t,
        };
        let bez = |a: f64, b: f64, u: f64| 3.0 * a * u * (1.0 - u).powi(2) + 3.0 * b * u * u * (1.0 - u) + u.powi(3);
        // The u where x(u) = t, by bisection (x rises with u).
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..40 {
            let mid = (lo + hi) / 2.0;
            if bez(x1, x2, mid) < t { lo = mid } else { hi = mid }
        }
        bez(y1, y2, (lo + hi) / 2.0)
    }
}

/// A spring's position `t` seconds in, from 0 toward 1.
fn spring_at(t: f64, response: f64, damping: f64) -> f64 {
    let w0 = std::f64::consts::TAU / response.max(0.01);
    if damping < 1.0 {
        let wd = w0 * (1.0 - damping * damping).sqrt();
        1.0 - (-damping * w0 * t).exp() * ((wd * t).cos() + damping * w0 / wd * (wd * t).sin())
    } else {
        1.0 - (-w0 * t).exp() * (1.0 + w0 * t)
    }
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

    /// A spring, as SwiftUI's: `response` is how quickly it moves (its
    /// period undamped, in seconds), `damping` how it settles: 1 without
    /// overshooting, less with a bounce (0.5 bounces a few times). It
    /// lasts until it's still. Things moved by a spring can be moved again
    /// mid-flight: they go on from where they are.
    pub fn spring(response: f64, damping: f64) -> Animation {
        let (response, damping) = (response.max(0.05), damping.clamp(0.05, 1.0));
        let w0 = std::f64::consts::TAU / response;
        // Until it's within a thousandth of where it's going.
        let settle = if damping < 1.0 { 6.9 / (damping * w0) } else { 9.3 / w0 };
        Animation { duration: settle.min(4.0), curve: Curve::Spring { response, damping } }
    }

    /// A spring with a visible bounce, about a tenth past the end
    /// (response 0.5, damping 0.6).
    pub fn bouncy() -> Animation {
        Animation::spring(0.5, 0.6)
    }

    /// A quick spring with a hint of bounce (response 0.3, damping 0.85).
    pub fn snappy() -> Animation {
        Animation::spring(0.3, 0.85)
    }

    /// A spring that settles without overshooting (response 0.5, damping 1).
    pub fn smooth() -> Animation {
        Animation::spring(0.5, 1.0)
    }

    /// How far along it is `seconds` in: 0 at the start, 1 at the end (a
    /// spring passes 1 and comes back).
    pub fn progress(&self, seconds: f64) -> f64 {
        match self.curve {
            Curve::Spring { response, damping } if seconds < self.duration => spring_at(seconds, response, damping),
            Curve::Spring { .. } => 1.0,
            c => c.eased(if self.duration > 0.0 { seconds / self.duration } else { 1.0 }),
        }
    }

    fn is_spring(&self) -> bool {
        matches!(self.curve, Curve::Spring { .. })
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
    static QUEUED: UiCell<Vec<Box<dyn FnOnce()>>> = UiCell::new(Vec::new());
    static ENTERING: UiCell<Vec<Entrance>> = const { UiCell::new(Vec::new()) };
    static EXITING: UiCell<Vec<(Vec<Retained<NSView>>, Transition)>> = const { UiCell::new(Vec::new()) };
    /// Views on their way out (removed when their animation ends).
    static LEAVING: Cell<usize> = const { Cell::new(0) };
    static REDUCE_MOTION: Cell<Option<bool>> = const { Cell::new(None) };
    /// The spring the running layout's frames follow, if it's one.
    static SPRING: Cell<Option<Animation>> = const { Cell::new(None) };
    /// Frames on their way under a spring, by view.
    static FRAMES: UiCell<std::collections::HashMap<usize, FrameJob>> = UiCell::new(std::collections::HashMap::new());
    /// Matched geometry: where views with an id that left were (window
    /// coordinates), and the views with one that came in.
    static MATCH_FROM: UiCell<std::collections::HashMap<String, NSRect>> = UiCell::new(std::collections::HashMap::new());
    static MATCH_TO: UiCell<Vec<(String, Vec<Retained<NSView>>)>> = const { UiCell::new(Vec::new()) };
    /// Views layout has placed (weakly, by address): a view not in it has
    /// never been placed, whatever its frame says (a label is created
    /// already sized to its text).
    static PLACED: UiCell<std::collections::HashMap<usize, objc2::rc::Weak<NSView>>> = UiCell::new(std::collections::HashMap::new());
}

fn key(view: &NSView) -> usize {
    view as *const NSView as usize
}

/// Whether layout has placed `view` yet.
fn placed(view: &NSView) -> bool {
    PLACED.with_borrow(|p| p.get(&key(view)).and_then(|w| w.load()).is_some_and(|v| std::ptr::eq(&*v, view)))
}

/// Records that layout placed `view`; true if it's the first time.
fn mark_placed(view: &NSView) -> bool {
    if placed(view) {
        return false;
    }
    PLACED.with_borrow_mut(|p| {
        // Swept when it has doubled since the last sweep, not on every
        // insert past a fixed size (thousands of live views are normal).
        if p.len() >= SWEEP_AT.with(Cell::get) {
            p.retain(|_, w| w.load().is_some());
            SWEEP_AT.with(|s| s.set((p.len() * 2).max(4096)));
        }
        p.insert(key(view), objc2::rc::Weak::from(view));
    });
    true
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
    let unplaced = mark_placed(view);
    // Under way to this frame already (a spring): leave it to finish.
    let flying = FRAMES.try_with(|f| f.borrow().get(&key(view)).map(|j| j.to)).ok().flatten();
    if flying == Some(frame) {
        return;
    }
    if flying.is_some() {
        FRAMES.with_borrow_mut(|f| f.remove(&key(view)));
    }
    match (RUNNING.with(Cell::get), SPRING.with(Cell::get)) {
        (true, _) if unplaced => without_animation(|| view.setFrame(frame)),
        (true, Some(spring)) if view.frame() != frame => fly(view, view.frame(), frame, spring),
        _ => view.setFrame(frame),
    }
}

/// A frame on its way under a spring.
struct FrameJob {
    view: objc2::rc::Weak<NSView>,
    from: NSRect,
    to: NSRect,
    start: f64,
    animation: Animation,
}

/// Moves `view` from `from` to `to` under `spring`, a frame at a time.
fn fly(view: &NSView, from: NSRect, to: NSRect, spring: Animation) {
    without_animation(|| view.setFrame(from));
    let job = FrameJob { view: objc2::rc::Weak::from(view), from, to, start: now(), animation: spring };
    FRAMES.with_borrow_mut(|f| f.insert(key(view), job));
    every_frame(FRAMES_KEY, Box::new(|_| {
        let t = now();
        let mut done = Vec::new();
        without_animation(|| {
            FRAMES.with_borrow(|frames| {
                for (k, j) in frames {
                    let Some(v) = j.view.load() else {
                        done.push(*k);
                        continue;
                    };
                    let elapsed = t - j.start;
                    if elapsed >= j.animation.duration {
                        v.setFrame(j.to);
                        done.push(*k);
                    } else {
                        v.setFrame(lerp(j.from, j.to, j.animation.progress(elapsed)));
                    }
                }
            });
        });
        FRAMES.with_borrow_mut(|f| {
            for k in done {
                f.remove(&k);
            }
            !f.is_empty()
        })
    }));
}

fn lerp(a: NSRect, b: NSRect, p: f64) -> NSRect {
    let l = |x: f64, y: f64| x + (y - x) * p;
    NSRect::new(NSPoint::new(l(a.origin.x, b.origin.x), l(a.origin.y, b.origin.y)), NSSize::new(l(a.size.width, b.size.width).max(0.0), l(a.size.height, b.size.height).max(0.0)))
}

/// The views with matched id `id` leaving: where they were.
pub(crate) fn matched_leaving(id: &str, views: &[Retained<NSView>]) {
    if !animating() {
        return;
    }
    if let Some(rect) = union_in_window(views) {
        let _ = MATCH_FROM.try_with(|m| m.borrow_mut().insert(id.to_string(), rect));
    }
}

/// The views with matched id `id` coming in (they start where the ones
/// that left were, if any did in this change).
pub(crate) fn matched_entering(id: &str, views: Vec<Retained<NSView>>) {
    if animating() {
        MATCH_TO.with_borrow_mut(|m| m.push((id.to_string(), views)));
    }
}

fn in_window(v: &NSView) -> Option<NSRect> {
    // SAFETY: reading the view's superview on the main thread.
    let sup = unsafe { v.superview() }?;
    Some(sup.convertRect_toView(v.frame(), None))
}

fn union_in_window(views: &[Retained<NSView>]) -> Option<NSRect> {
    views.iter().filter_map(|v| in_window(v)).reduce(|a, b| {
        let (x0, y0) = (a.origin.x.min(b.origin.x), a.origin.y.min(b.origin.y));
        let (x1, y1) = ((a.origin.x + a.size.width).max(b.origin.x + b.size.width), (a.origin.y + a.size.height).max(b.origin.y + b.size.height));
        NSRect::new(NSPoint::new(x0, y0), NSSize::new(x1 - x0, y1 - y0))
    })
}

/// Starts the matched views that came in where their partners were, and
/// moves them to their places (inside the running animation).
fn run_matched(reduce: bool, spring: Option<Animation>) {
    let arrivals = MATCH_TO.with_borrow_mut(std::mem::take);
    let mut from = MATCH_FROM.with_borrow_mut(std::mem::take);
    if reduce {
        return;
    }
    for (id, views) in arrivals {
        let (Some(old), Some(new)) = (from.remove(&id), union_in_window(&views)) else { continue };
        let (sx, sy) = (old.size.width / new.size.width.max(1.0), old.size.height / new.size.height.max(1.0));
        for v in &views {
            let (Some(sup), Some(target)) = (unsafe { v.superview() }, in_window(v)) else { continue };
            let start = NSRect::new(
                NSPoint::new(old.origin.x + (target.origin.x - new.origin.x) * sx, old.origin.y + (target.origin.y - new.origin.y) * sy),
                NSSize::new(target.size.width * sx, target.size.height * sy),
            );
            let start = sup.convertRect_fromView(start, None);
            let end = v.frame();
            match spring {
                Some(s) => fly(v, start, end, s),
                None => {
                    without_animation(|| v.setFrame(start));
                    v.setFrame(end);
                }
            }
        }
    }
}

// ---- the driver: work a frame at a time ----

#[link(name = "QuartzCore", kind = "framework")]
unsafe extern "C" {
    fn CACurrentMediaTime() -> f64;
}

const FRAMES_KEY: u64 = u64::MAX;

/// Work run each frame until it says it's done.
type Step = Box<dyn FnMut(f64) -> bool>;

thread_local! {
    static JOBS: UiCell<Vec<(u64, f64, Step)>> = const { UiCell::new(Vec::new()) };
    /// Jobs stopped since the current frame began (see [`stop_animation`]).
    static STOPPED: UiCell<Vec<u64>> = const { UiCell::new(Vec::new()) };
    /// The size of `PLACED` at which it is next swept.
    static SWEEP_AT: Cell<usize> = const { Cell::new(4096) };
    static TIMER: UiCell<Option<Retained<AnyObject>>> = const { UiCell::new(None) };
    /// Time added to the clock by [`step`], for checks.
    static SKEW: Cell<f64> = const { Cell::new(0.0) };
}

/// The clock animations run on, in seconds.
fn now() -> f64 {
    // SAFETY: Core Animation's media clock.
    (unsafe { CACurrentMediaTime() }) + SKEW.with(Cell::get)
}

define_class!(
    // The timer's target: runs the frame's work.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FerrokitMotionTicker"]
    struct Ticker;

    impl Ticker {
        #[unsafe(method(tick:))]
        fn tick(&self, _timer: &AnyObject) {
            crate::failure::guard("an animation frame (tick)", (), || {
                tick();
            })
        }
    }
);

/// Runs `step` every frame (about 120 a second) until it returns false;
/// one with the same `key` running already is replaced.
fn every_frame(key: u64, step: Step) {
    let start = now();
    JOBS.with_borrow_mut(|j| {
        j.retain(|(k, ..)| *k != key);
        j.push((key, start, step));
    });
    if TIMER.with_borrow(Option::is_none) {
        let mtm = crate::mtm();
        let target = Ticker::alloc(mtm).set_ivars(());
        // SAFETY: NSObject's designated initialiser; the timer retains the
        // target until it's invalidated, and runs in the common modes (on
        // during live resize and tracking).
        let timer: Retained<AnyObject> = unsafe {
            let target: Retained<Ticker> = msg_send![super(target), init];
            let timer: Retained<AnyObject> = msg_send![objc2::class!(NSTimer), timerWithTimeInterval: 1.0 / 120.0, target: &*target, selector: sel!(tick:), userInfo: Option::<&AnyObject>::None, repeats: true];
            let run_loop: Retained<AnyObject> = msg_send![objc2::class!(NSRunLoop), mainRunLoop];
            let () = msg_send![&*run_loop, addTimer: &*timer, forMode: objc2_foundation::NSRunLoopCommonModes];
            timer
        };
        TIMER.with_borrow_mut(|t| *t = Some(timer));
    }
}

/// One frame of every running animation.
fn tick() {
    let t = now();
    let running = JOBS.with_borrow_mut(std::mem::take);
    STOPPED.with_borrow_mut(Vec::clear);
    let mut kept = Vec::new();
    crate::failure::entry("an animation frame", (), || {
        for (key, start, mut step) in running {
            if step(t - start) {
                kept.push((key, start, step));
            }
        }
    });
    JOBS.with_borrow_mut(|j| {
        // Work added during this frame replaces what it was keyed with.
        let stopped = STOPPED.with_borrow_mut(std::mem::take);
        kept.retain(|(k, ..)| !j.iter().any(|(n, ..)| n == k) && !stopped.contains(k));
        j.extend(kept);
        if j.is_empty() {
            TIMER.with_borrow_mut(|timer| {
                if let Some(timer) = timer.take() {
                    // SAFETY: NSTimer's invalidate (releasing the target).
                    let () = unsafe { msg_send![&*timer, invalidate] };
                }
            });
        }
    });
}

/// Moves the animation clock on by `seconds` and runs a frame (for checks:
/// animations land where they'd be that much later).
pub(crate) fn step(seconds: f64) {
    SKEW.with(|s| s.set(s.get() + seconds));
    tick();
}

/// Animations running now (for checks).
pub(crate) fn running() -> usize {
    JOBS.with_borrow(Vec::len)
}

/// Animates `value` from where it is to `to` with `animation`, setting it
/// every frame (about 120 times a second): views and drawings that read it
/// follow. A spring passes `to` and comes back; a new animation of the same
/// signal takes over from where it is. With Reduce Motion on, it's set to
/// `to` at once.
///
/// ```no_run
/// use ferrokit::*;
///
/// let level = Signal::new(0.0);
/// let fill = button("Fill", move || animate(&level, 1.0, Animation::bouncy()));
/// let gauge = canvas(move |p| p.fill_rect(0.0, 0.0, p.width() * level.get(), p.height(), Color::Green));
/// # let _ = (fill, gauge);
/// ```
pub fn animate(value: &Signal<f64>, to: f64, animation: Animation) {
    keyframes(value, Keyframes { start: None, steps: vec![(to, animation)], repeat: false });
}

/// Values a signal goes through over time, each reached with its own
/// animation (see [`keyframes`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Keyframes {
    start: Option<f64>,
    steps: Vec<(f64, Animation)>,
    repeat: bool,
}

impl Keyframes {
    /// Keyframes starting at `value` (or, with [`from_current`](Self::from_current),
    /// wherever the signal is).
    pub fn new(value: f64) -> Keyframes {
        Keyframes { start: Some(value), steps: Vec::new(), repeat: false }
    }

    /// Keyframes starting wherever the signal is.
    pub fn from_current() -> Keyframes {
        Keyframes { start: None, steps: Vec::new(), repeat: false }
    }

    /// Then to `value` over `seconds`, easing in and out.
    pub fn to(self, value: f64, seconds: f64) -> Keyframes {
        self.with(value, Animation::ease_in_out(seconds))
    }

    /// Then to `value` at a constant pace over `seconds`.
    pub fn linear(self, value: f64, seconds: f64) -> Keyframes {
        self.with(value, Animation::linear(seconds))
    }

    /// Then to `value` with `animation` (a spring, say).
    pub fn with(mut self, value: f64, animation: Animation) -> Keyframes {
        self.steps.push((value, animation));
        self
    }

    /// Over again from the start when it ends, until [`stop_animation`]
    /// (or the signal goes).
    pub fn repeating(mut self) -> Keyframes {
        self.repeat = true;
        self
    }

    /// How long one run takes, in seconds.
    pub fn duration(&self) -> f64 {
        self.steps.iter().map(|(_, a)| a.duration).sum()
    }
}

/// Takes `value` through `frames` (SwiftUI's keyframe animator): a pulse,
/// a shake, a count-up, a bounce; each step reached with its own timing.
/// Reduce Motion skips to the last value (a repeating one stays at its
/// start).
///
/// ```no_run
/// use ferrokit::*;
///
/// // A nudge: right, left, and back to rest; a canvas draws it offset.
/// let shake = Signal::new(0.0);
/// let card = canvas(move |p| p.fill_rounded_rect(shake.get(), 0.0, p.width(), p.height(), 8.0, Color::Red));
/// keyframes(&shake, Keyframes::new(0.0).to(8.0, 0.08).to(-6.0, 0.1).with(0.0, Animation::snappy()));
/// # let _ = card;
/// ```
pub fn keyframes(value: &Signal<f64>, frames: Keyframes) {
    let value = *value;
    if !value.is_alive() || frames.steps.is_empty() {
        return;
    }
    if reduce_motion() {
        let rest = if frames.repeat { frames.start.unwrap_or_else(|| value.get_untracked()) } else { frames.steps.last().map(|s| s.0).unwrap_or(0.0) };
        value.set(rest);
        return;
    }
    let start = frames.start.unwrap_or_else(|| value.get_untracked());
    let total = frames.duration().max(1e-6);
    every_frame(value.key(), Box::new(move |elapsed| {
        if !value.is_alive() {
            return false;
        }
        let mut t = if frames.repeat { elapsed % total } else { elapsed };
        let mut from = start;
        for (to, animation) in &frames.steps {
            if t < animation.duration {
                value.set(from + (to - from) * animation.progress(t));
                return true;
            }
            t -= animation.duration;
            from = *to;
        }
        value.set(from);
        frames.repeat
    }));
}

/// Stops animating `value` (where it is now).
pub fn stop_animation(value: &Signal<f64>) {
    let key = value.key();
    JOBS.with_borrow_mut(|j| j.retain(|(k, ..)| *k != key));
    // Called from inside a frame (an effect watching the value), the job
    // is out of the list while it runs: noted, so it isn't put back.
    STOPPED.with_borrow_mut(|s| s.push(key));
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
    // A spring moves frames itself; fades and colours ease out alongside.
    let spring = animation.is_spring().then_some(animation);
    cx.setDuration(match animation.curve {
        Curve::Spring { response, .. } => response * 0.7,
        _ => animation.duration,
    });
    cx.setAllowsImplicitAnimation(true);
    // SAFETY: Core Animation's own timing function names.
    let name = unsafe {
        use objc2_quartz_core::*;
        match animation.curve {
            Curve::EaseInOut => kCAMediaTimingFunctionEaseInEaseOut,
            Curve::EaseOut | Curve::Spring { .. } => kCAMediaTimingFunctionEaseOut,
            Curve::EaseIn => kCAMediaTimingFunctionEaseIn,
            Curve::Linear => kCAMediaTimingFunctionLinear,
        }
    };
    cx.setTimingFunction(Some(&objc2_quartz_core::CAMediaTimingFunction::functionWithName(name)));
    RUNNING.with(|r| r.set(true));
    SPRING.with(|s| s.set(if reduce { None } else { spring }));
    if let Some(l) = layout.take() {
        l();
    }
    run_matched(reduce, spring);
    for change in QUEUED.with_borrow_mut(std::mem::take) {
        change();
    }
    for e in ENTERING.with_borrow_mut(std::mem::take) {
        // Slides start off by a little toward their edge.
        if let (Transition::Slide(edge), false) = (e.transition, reduce) {
            // Only views layout has placed: one placed later (inside a
            // scroll view whose content lays out on the next pass) would
            // otherwise animate to where it was made, and stay there.
            for v in e.views.iter().filter(|v| placed(v)) {
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
    SPRING.with(|s| s.set(None));
    NSAnimationContext::endGrouping();
}

/// `frame` moved toward `edge` (the views' containers are flipped: down is
/// +y).
fn offset(mut frame: NSRect, edge: Edge) -> NSRect {
    // Leading is the right edge right to left.
    let edge = match (edge, crate::right_to_left()) {
        (Edge::Leading, true) => Edge::Trailing,
        (Edge::Trailing, true) => Edge::Leading,
        (e, _) => e,
    };
    match edge {
        Edge::Above => frame.origin.y -= SLIDE,
        Edge::Below => frame.origin.y += SLIDE,
        Edge::Leading => frame.origin.x -= SLIDE,
        Edge::Trailing => frame.origin.x += SLIDE,
    }
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn springs_settle() {
        let bouncy = Animation::bouncy();
        assert_eq!(bouncy.progress(0.0), 0.0);
        assert_eq!(bouncy.progress(bouncy.duration()), 1.0);
        // It overshoots, then comes back to rest.
        let peak = (1..200).map(|i| bouncy.progress(i as f64 * 0.01)).fold(0.0, f64::max);
        assert!(peak > 1.05 && peak < 1.3, "peak {peak}");
        assert!((bouncy.progress(bouncy.duration() - 0.01) - 1.0).abs() < 0.01);
        // Critically damped: never past the end.
        let smooth = Animation::smooth();
        assert!((1..400).all(|i| smooth.progress(i as f64 * 0.01) <= 1.0 + 1e-9));
        assert!(Animation::snappy().duration() < bouncy.duration());
    }

    #[test]
    fn easings_meet_their_ends() {
        for a in [Animation::ease_in_out(1.0), Animation::ease_in(1.0), Animation::ease_out(1.0), Animation::linear(1.0)] {
            assert!(a.progress(0.0).abs() < 1e-6 && (a.progress(1.0) - 1.0).abs() < 1e-6, "{a:?}");
        }
        // Ease out is ahead of linear early on; ease in behind it.
        assert!(Animation::ease_out(1.0).progress(0.25) > 0.25 && Animation::ease_in(1.0).progress(0.25) < 0.25);
        assert!((Animation::ease_in_out(1.0).progress(0.5) - 0.5).abs() < 1e-3);
    }
}
