//! Fine-grained, main-thread reactive state.
//!
//! An [`Effect`] remembers which signals it read during its last run;
//! changing one of them queues the effect. Everything runs inside a
//! [`batch`]: effects run once, when the outermost batch ends, so an effect
//! never re-enters itself and several changes cost one run. Views (F1.3) own
//! the effects that update their native views.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::{Rc, Weak};

struct EffectInner {
    f: RefCell<Box<dyn FnMut()>>,
    /// Bumped on every run; subscriptions from older runs are stale.
    generation: Cell<u64>,
    queued: Cell<bool>,
    stopped: Cell<bool>,
}

thread_local! {
    static OBSERVER: RefCell<Option<Rc<EffectInner>>> = const { RefCell::new(None) };
    static DEPTH: Cell<u32> = const { Cell::new(0) };
    static PENDING: RefCell<Vec<Rc<EffectInner>>> = const { RefCell::new(Vec::new()) };
    static AFTER: RefCell<Vec<Rc<dyn Fn()>>> = const { RefCell::new(Vec::new()) };
    static RUNS: Cell<u64> = const { Cell::new(0) };
}

impl EffectInner {
    fn run(self: &Rc<Self>) {
        if self.stopped.get() {
            return;
        }
        RUNS.with(|r| r.set(r.get() + 1));
        self.generation.set(self.generation.get() + 1);
        let prev = OBSERVER.with_borrow_mut(|o| o.replace(self.clone()));
        (self.f.borrow_mut())();
        OBSERVER.with_borrow_mut(|o| *o = prev);
    }
}

/// A side effect that re-runs when the signals it read change. It stops when
/// the handle is dropped.
#[must_use = "an effect stops when its handle is dropped"]
pub struct Effect(Rc<EffectInner>);

impl Effect {
    /// Runs `f` now, and again (once per batch) whenever a signal it read
    /// during its last run changes.
    pub fn new(f: impl FnMut() + 'static) -> Effect {
        let inner = Rc::new(EffectInner {
            f: RefCell::new(Box::new(f)),
            generation: Cell::new(0),
            queued: Cell::new(false),
            stopped: Cell::new(false),
        });
        inner.run();
        Effect(inner)
    }
}

impl Drop for Effect {
    fn drop(&mut self) {
        self.0.stopped.set(true);
    }
}

impl fmt::Debug for Effect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Effect").field("runs", &self.0.generation.get()).finish()
    }
}

/// Total effect runs on this thread so far (diagnostic).
pub fn effect_runs() -> u64 {
    RUNS.with(Cell::get)
}

/// Runs `f`; effects triggered inside run once, when the outermost batch ends.
///
/// The outermost batch also drains its own autorelease pool, so a burst of
/// updates (each creating and removing native views) doesn't pile up
/// autoreleased AppKit objects until the run loop's next pass.
pub fn batch<R>(f: impl FnOnce() -> R) -> R {
    if DEPTH.with(Cell::get) > 0 {
        return nested(f);
    }
    use crate::platform::{Native, Platform};
    Native::scope(|| nested(f))
}

fn nested<R>(f: impl FnOnce() -> R) -> R {
    DEPTH.with(|d| d.set(d.get() + 1));
    let r = f();
    if DEPTH.with(Cell::get) == 1 {
        flush();
    }
    DEPTH.with(|d| d.set(d.get() - 1));
    r
}

/// Registers `f` to run after every outermost batch, once its effects have
/// run (the layout pass hooks in here).
pub fn after_batch(f: impl Fn() + 'static) {
    AFTER.with_borrow_mut(|a| a.push(Rc::new(f)));
}

/// Reads signals inside `f` without subscribing the running effect to them.
pub fn untrack<R>(f: impl FnOnce() -> R) -> R {
    let prev = OBSERVER.with_borrow_mut(Option::take);
    let r = f();
    OBSERVER.with_borrow_mut(|o| *o = prev);
    r
}

fn flush() {
    loop {
        while let Some(e) = PENDING.with_borrow_mut(|p| (!p.is_empty()).then(|| p.remove(0))) {
            e.queued.set(false);
            e.run();
        }
        let hooks: Vec<_> = AFTER.with_borrow(|a| a.clone());
        for h in hooks {
            h();
        }
        if PENDING.with_borrow(Vec::is_empty) {
            break;
        }
    }
}

/// Mutable state. Cloning gives another handle to the same value.
pub struct Signal<T>(Rc<SignalInner<T>>);

struct SignalInner<T> {
    value: RefCell<T>,
    subs: RefCell<Vec<(Weak<EffectInner>, u64)>>,
    /// Subscriber count at which `track` next sweeps out dead and outdated
    /// entries (`notify` drops them all, but a signal that rarely changes,
    /// read by views that come and go, would otherwise keep every one).
    sweep_at: Cell<usize>,
}

/// The smallest subscriber list `track` sweeps.
const SWEEP_MIN: usize = 32;

impl<T> Clone for Signal<T> {
    fn clone(&self) -> Self {
        Signal(self.0.clone())
    }
}

impl<T: fmt::Debug> fmt::Debug for Signal<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Signal").field(&self.0.value.borrow()).finish()
    }
}

impl<T: 'static> Signal<T> {
    /// A signal holding `value`.
    pub fn new(value: T) -> Self {
        Signal(Rc::new(SignalInner { value: RefCell::new(value), subs: RefCell::new(Vec::new()), sweep_at: Cell::new(SWEEP_MIN) }))
    }

    fn track(&self) {
        OBSERVER.with_borrow(|o| {
            if let Some(e) = o {
                let generation = e.generation.get();
                let mut subs = self.0.subs.borrow_mut();
                if !subs.iter().any(|(w, g)| *g == generation && std::ptr::eq(w.as_ptr(), Rc::as_ptr(e))) {
                    if subs.len() >= self.0.sweep_at.get() {
                        // Keep live effects still on the run that read this
                        // signal; doubling the threshold keeps sweeps amortised O(1).
                        subs.retain(|(w, g)| w.upgrade().is_some_and(|e| e.generation.get() == *g && !e.stopped.get()));
                        self.0.sweep_at.set((subs.len() * 2).max(SWEEP_MIN));
                    }
                    subs.push((Rc::downgrade(e), generation));
                }
            }
        });
    }

    /// Reads the value through `f`, subscribing the running effect.
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        self.track();
        f(&self.0.value.borrow())
    }

    /// A copy of the value, subscribing the running effect.
    pub fn get(&self) -> T
    where
        T: Clone,
    {
        self.with(T::clone)
    }

    /// A copy of the value, without subscribing.
    pub fn get_untracked(&self) -> T
    where
        T: Clone,
    {
        self.0.value.borrow().clone()
    }

    /// Replaces the value and notifies readers.
    pub fn set(&self, value: T) {
        *self.0.value.borrow_mut() = value;
        self.notify();
    }

    /// Replaces the value and notifies readers only if it changed.
    pub fn set_if_changed(&self, value: T)
    where
        T: PartialEq,
    {
        if *self.0.value.borrow() != value {
            self.set(value);
        }
    }

    /// Changes the value in place and notifies readers.
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        f(&mut self.0.value.borrow_mut());
        self.notify();
    }

    fn notify(&self) {
        // Effects subscribe again when they re-run, so take them all.
        let subs = std::mem::take(&mut *self.0.subs.borrow_mut());
        batch(|| {
            for (w, g) in subs {
                if let Some(e) = w.upgrade().filter(|e| e.generation.get() == g && !e.stopped.get())
                    && !e.queued.replace(true)
                {
                    PENDING.with_borrow_mut(|p| p.push(e));
                }
            }
        });
    }
}

/// Derived state: recomputed when its inputs change; readers are notified
/// only when the value actually changes.
pub struct Memo<T> {
    signal: Signal<T>,
    _effect: Rc<Effect>,
}

impl<T> Clone for Memo<T> {
    fn clone(&self) -> Self {
        Memo { signal: self.signal.clone(), _effect: self._effect.clone() }
    }
}

impl<T: PartialEq + Clone + 'static> Memo<T> {
    /// A value computed by `f` from other signals.
    pub fn new(f: impl Fn() -> T + 'static) -> Self {
        let cell: Rc<RefCell<Option<Signal<T>>>> = Rc::new(RefCell::new(None));
        let c = cell.clone();
        let effect = Effect::new(move || {
            let v = f();
            let existing = c.borrow().clone();
            match existing {
                None => *c.borrow_mut() = Some(Signal::new(v)),
                Some(s) => s.set_if_changed(v),
            }
        });
        let signal = cell.borrow().clone().expect("memo computed");
        Memo { signal, _effect: Rc::new(effect) }
    }

    /// The current value, subscribing the running effect.
    pub fn get(&self) -> T {
        self.signal.get()
    }

    /// Reads the value through `f`, subscribing the running effect.
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        self.signal.with(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counter() -> (Rc<Cell<u32>>, impl Fn() + Clone) {
        let n = Rc::new(Cell::new(0));
        let m = n.clone();
        (n, move || m.set(m.get() + 1))
    }

    #[test]
    fn effect_runs_on_change_and_stops_on_drop() {
        let s = Signal::new(1);
        let (runs, bump) = counter();
        let seen = Rc::new(Cell::new(0));
        let (s2, seen2) = (s.clone(), seen.clone());
        let e = Effect::new(move || {
            bump();
            seen2.set(s2.get());
        });
        assert_eq!((runs.get(), seen.get()), (1, 1));
        s.set(2);
        assert_eq!((runs.get(), seen.get()), (2, 2));
        drop(e);
        s.set(3);
        assert_eq!(runs.get(), 2);
    }

    #[test]
    fn batch_coalesces() {
        let (a, b) = (Signal::new(0), Signal::new(0));
        let (runs, bump) = counter();
        let (a2, b2) = (a.clone(), b.clone());
        let _e = Effect::new(move || {
            bump();
            let _ = a2.get() + b2.get();
        });
        batch(|| {
            a.set(1);
            b.set(1);
            a.set(2);
        });
        assert_eq!(runs.get(), 2);
    }

    #[test]
    fn dynamic_dependencies() {
        let (flag, x, y) = (Signal::new(true), Signal::new(0), Signal::new(0));
        let (runs, bump) = counter();
        let (f2, x2, y2) = (flag.clone(), x.clone(), y.clone());
        let _e = Effect::new(move || {
            bump();
            let _ = if f2.get() { x2.get() } else { y2.get() };
        });
        y.set(1); // not read yet
        assert_eq!(runs.get(), 1);
        flag.set(false);
        assert_eq!(runs.get(), 2);
        x.set(1); // no longer read
        assert_eq!(runs.get(), 2);
        y.set(2);
        assert_eq!(runs.get(), 3);
    }

    #[test]
    fn memo_notifies_only_on_change() {
        let n = Signal::new(4);
        let n2 = n.clone();
        let even = Memo::new(move || n2.get() % 2 == 0);
        let (runs, bump) = counter();
        let e2 = even.clone();
        let _e = Effect::new(move || {
            bump();
            let _ = e2.get();
        });
        n.set(6); // still even
        assert_eq!(runs.get(), 1);
        n.set(7);
        assert_eq!(runs.get(), 2);
        assert!(!even.get());
    }

    #[test]
    fn diamond_runs_once() {
        let a = Signal::new(1);
        let (a1, a2) = (a.clone(), a.clone());
        let b = Memo::new(move || a1.get() + 1);
        let c = Memo::new(move || a2.get() * 2);
        let (runs, bump) = counter();
        let (b2, c2) = (b.clone(), c.clone());
        let total = Rc::new(Cell::new(0));
        let t2 = total.clone();
        let _e = Effect::new(move || {
            bump();
            t2.set(b2.get() + c2.get());
        });
        a.set(10);
        assert_eq!(total.get(), 31);
        assert_eq!(runs.get(), 2);
    }

    #[test]
    fn effects_can_set_signals() {
        let (src, dst) = (Signal::new(1), Signal::new(0));
        let (s2, d2) = (src.clone(), dst.clone());
        let _copy = Effect::new(move || d2.set(s2.get() * 10));
        assert_eq!(dst.get_untracked(), 10);
        src.set(2);
        assert_eq!(dst.get_untracked(), 20);
    }

    #[test]
    fn untrack_and_set_if_changed() {
        let s = Signal::new(1);
        let (runs, bump) = counter();
        let s2 = s.clone();
        let _e = Effect::new(move || {
            bump();
            untrack(|| s2.get());
        });
        s.set(2);
        assert_eq!(runs.get(), 1);
        let t = Signal::new(5);
        let t2 = t.clone();
        let (runs, bump) = counter();
        let _e = Effect::new(move || {
            bump();
            let _ = t2.get();
        });
        t.set_if_changed(5);
        assert_eq!(runs.get(), 1);
    }

    #[test]
    fn short_lived_readers_dont_pile_up() {
        // A signal that never changes, read by effects that come and go (views
        // in windows that open and close): its list stays small.
        let s = Signal::new(0);
        let keep = Signal::new(0);
        let (k, (runs, bump)) = (keep.clone(), counter());
        let _kept = Effect::new(move || {
            bump();
            let _ = k.get();
        });
        for _ in 0..10_000 {
            let s2 = s.clone();
            let e = Effect::new(move || {
                let _ = s2.get();
            });
            drop(e);
        }
        assert!(s.0.subs.borrow().len() <= 2 * SWEEP_MIN, "{} entries", s.0.subs.borrow().len());
        // Live readers survive sweeps and still run.
        let live: Vec<Effect> = (0..100)
            .map(|_| {
                let s2 = s.clone();
                Effect::new(move || {
                    let _ = s2.get();
                })
            })
            .collect();
        for _ in 0..1_000 {
            let s2 = s.clone();
            drop(Effect::new(move || {
                let _ = s2.get();
            }));
        }
        let before = effect_runs();
        s.set(1);
        assert_eq!(effect_runs() - before, 100);
        keep.set(1);
        assert_eq!(runs.get(), 2);
        drop(live);
    }
}
