//! Fine-grained, main-thread reactive state.
//!
//! An [`Effect`] remembers which signals it read during its last run;
//! changing one of them queues the effect. Everything runs inside a
//! [`batch`]: effects run once, when the outermost batch ends, so an effect
//! never re-enters itself and several changes cost one run. Views own the
//! effects that update their native views.
//!
//! [`Signal`] and [`Memo`] are `Copy` handles into a per-thread arena, so
//! closures capture them without cloning. Each belongs to the *owner* that
//! was current when it was created: a window's view tree, a list row, a
//! [`dynamic`](crate::dynamic) branch or a document. It's freed with that
//! owner. Signals created outside any owner (app state made in `main`) live
//! as long as the app.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::fmt;
use std::marker::PhantomData;
use std::rc::{Rc, Weak};

struct EffectInner {
    f: RefCell<Box<dyn FnMut()>>,
    /// Bumped on every run; subscriptions from older runs are stale.
    generation: Cell<u64>,
    queued: Cell<bool>,
    stopped: Cell<bool>,
    /// The owner current when the effect was made: current again while it
    /// runs, so signals it creates belong there.
    owner: Option<Weak<OwnerInner>>,
}

// ---- owners and the arena ----

/// Frees the signals (and memos' effects) created while it was current,
/// when the last clone is dropped. Nodes, windows and documents hold one.
#[derive(Clone)]
pub(crate) struct Owner(Rc<OwnerInner>);

#[derive(Default)]
pub(crate) struct OwnerInner {
    signals: RefCell<Vec<(u32, u32)>>,
    effects: RefCell<Vec<Effect>>,
}

impl Owner {
    pub(crate) fn new() -> Owner {
        Owner(Rc::new(OwnerInner::default()))
    }

    /// Runs `f` with this owner current.
    pub(crate) fn with<R>(&self, f: impl FnOnce() -> R) -> R {
        let prev = OWNER.with_borrow_mut(|o| o.replace(self.0.clone()));
        let r = f();
        OWNER.with_borrow_mut(|o| *o = prev);
        r
    }
}

impl Drop for OwnerInner {
    fn drop(&mut self) {
        // Memos' effects stop first, then the signals go.
        self.effects.borrow_mut().clear();
        let signals = std::mem::take(&mut *self.signals.borrow_mut());
        // The values are dropped outside the arena's borrow (a value may
        // own another owner).
        let freed: Vec<Rc<dyn Any>> = ARENA.with_borrow_mut(|a| signals.iter().filter_map(|&(i, g)| a.free(i, g)).collect());
        drop(freed);
    }
}

struct Slot {
    value: Option<Rc<dyn Any>>,
    generation: u32,
}

#[derive(Default)]
struct Arena {
    slots: Vec<Slot>,
    free: Vec<u32>,
}

impl Arena {
    fn insert(&mut self, value: Rc<dyn Any>) -> (u32, u32) {
        match self.free.pop() {
            Some(i) => {
                let slot = &mut self.slots[i as usize];
                slot.value = Some(value);
                (i, slot.generation)
            }
            None => {
                self.slots.push(Slot { value: Some(value), generation: 0 });
                ((self.slots.len() - 1) as u32, 0)
            }
        }
    }

    fn free(&mut self, index: u32, generation: u32) -> Option<Rc<dyn Any>> {
        let slot = self.slots.get_mut(index as usize)?;
        if slot.generation != generation {
            return None;
        }
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(index);
        slot.value.take()
    }
}

/// `f`, run with the owner current now (for callbacks registered while
/// building: signals a button's action makes, say for a sheet's form,
/// belong to the button's view tree rather than living for the app).
pub(crate) fn with_current_owner(f: impl Fn() + 'static) -> impl Fn() + 'static {
    let owner = OWNER.with_borrow(|o| o.as_ref().map(Rc::downgrade));
    move || match owner.as_ref().and_then(Weak::upgrade) {
        Some(o) => Owner(o).with(&f),
        None => f(),
    }
}

/// Live signals in the arena (diagnostic: freed ones are reused).
pub fn live_signals() -> usize {
    ARENA.with_borrow(|a| a.slots.len() - a.free.len())
}

thread_local! {
    static OBSERVER: RefCell<Option<Rc<EffectInner>>> = const { RefCell::new(None) };
    static DEPTH: Cell<u32> = const { Cell::new(0) };
    static PENDING: RefCell<Vec<Rc<EffectInner>>> = const { RefCell::new(Vec::new()) };
    static AFTER: RefCell<Vec<Rc<dyn Fn()>>> = const { RefCell::new(Vec::new()) };
    static RUNS: Cell<u64> = const { Cell::new(0) };
    static OWNER: RefCell<Option<Rc<OwnerInner>>> = const { RefCell::new(None) };
    static ARENA: RefCell<Arena> = RefCell::new(Arena::default());
}

impl EffectInner {
    fn run(self: &Rc<Self>) {
        if self.stopped.get() {
            return;
        }
        RUNS.with(|r| r.set(r.get() + 1));
        self.generation.set(self.generation.get() + 1);
        let prev = OBSERVER.with_borrow_mut(|o| o.replace(self.clone()));
        // The effect's own owner is current (none for app-level effects),
        // whatever was current where the batch flushed.
        let owner = self.owner.as_ref().and_then(Weak::upgrade);
        let prev_owner = OWNER.with_borrow_mut(|c| std::mem::replace(c, owner));
        (self.f.borrow_mut())();
        OWNER.with_borrow_mut(|c| *c = prev_owner);
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
            owner: OWNER.with_borrow(|o| o.as_ref().map(Rc::downgrade)),
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

/// `Copy` for any `T` and `!Send` (main-thread only).
type Handle<T> = (fn() -> T, *const ());

/// Mutable state: a `Copy` handle (closures capture it as is). It belongs
/// to the owner current when it was made (see the module docs) and is freed
/// with it; afterwards reading it panics and setting it does nothing.
pub struct Signal<T> {
    index: u32,
    generation: u32,
    _marker: PhantomData<Handle<T>>,
}

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
        *self
    }
}

impl<T> Copy for Signal<T> {}

impl<T> PartialEq for Signal<T> {
    fn eq(&self, other: &Self) -> bool {
        (self.index, self.generation) == (other.index, other.generation)
    }
}

impl<T> Eq for Signal<T> {}

impl<T: fmt::Debug + 'static> fmt::Debug for Signal<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.try_inner() {
            Some(inner) => f.debug_tuple("Signal").field(&inner.value.borrow()).finish(),
            None => f.write_str("Signal(<freed>)"),
        }
    }
}

impl<T: 'static> Signal<T> {
    /// A signal holding `value`, owned by the current owner (if any).
    pub fn new(value: T) -> Self {
        let inner: Rc<dyn Any> = Rc::new(SignalInner { value: RefCell::new(value), subs: RefCell::new(Vec::new()), sweep_at: Cell::new(SWEEP_MIN) });
        let (index, generation) = ARENA.with_borrow_mut(|a| a.insert(inner));
        OWNER.with_borrow(|o| {
            if let Some(o) = o {
                o.signals.borrow_mut().push((index, generation));
            }
        });
        Signal { index, generation, _marker: PhantomData }
    }

    fn try_inner(&self) -> Option<Rc<SignalInner<T>>> {
        let value = ARENA.with_borrow(|a| {
            let slot = a.slots.get(self.index as usize)?;
            (slot.generation == self.generation).then(|| slot.value.clone()).flatten()
        })?;
        value.downcast::<SignalInner<T>>().ok()
    }

    fn inner(&self) -> Rc<SignalInner<T>> {
        self.try_inner().expect(
            "signal read after it was freed: it belonged to a window, list row, dynamic branch or document that has closed",
        )
    }

    /// Whether the signal still exists (its owner hasn't been dropped).
    pub fn is_alive(&self) -> bool {
        self.try_inner().is_some()
    }

    /// A copy of the value, subscribing the running effect; `None` once the
    /// signal was freed.
    pub fn try_get(&self) -> Option<T>
    where
        T: Clone,
    {
        self.try_inner()?;
        Some(self.get())
    }

    fn track(inner: &SignalInner<T>) {
        OBSERVER.with_borrow(|o| {
            if let Some(e) = o {
                let generation = e.generation.get();
                let mut subs = inner.subs.borrow_mut();
                if !subs.iter().any(|(w, g)| *g == generation && std::ptr::eq(w.as_ptr(), Rc::as_ptr(e))) {
                    if subs.len() >= inner.sweep_at.get() {
                        // Keep live effects still on the run that read this
                        // signal; doubling the threshold keeps sweeps amortised O(1).
                        subs.retain(|(w, g)| w.upgrade().is_some_and(|e| e.generation.get() == *g && !e.stopped.get()));
                        inner.sweep_at.set((subs.len() * 2).max(SWEEP_MIN));
                    }
                    subs.push((Rc::downgrade(e), generation));
                }
            }
        });
    }

    /// Reads the value through `f`, subscribing the running effect.
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        let inner = self.inner();
        Self::track(&inner);
        f(&inner.value.borrow())
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
        self.inner().value.borrow().clone()
    }

    /// Replaces the value and notifies readers (nothing, once freed).
    pub fn set(&self, value: T) {
        let Some(inner) = self.try_inner() else { return };
        *inner.value.borrow_mut() = value;
        Self::notify(&inner);
    }

    /// Replaces the value and notifies readers only if it changed.
    pub fn set_if_changed(&self, value: T)
    where
        T: PartialEq,
    {
        let Some(inner) = self.try_inner() else { return };
        if *inner.value.borrow() != value {
            *inner.value.borrow_mut() = value;
            Self::notify(&inner);
        }
    }

    /// Changes the value in place and notifies readers (nothing, once freed).
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        let Some(inner) = self.try_inner() else { return };
        f(&mut inner.value.borrow_mut());
        Self::notify(&inner);
    }

    fn notify(inner: &SignalInner<T>) {
        // Effects subscribe again when they re-run, so take them all.
        let subs = std::mem::take(&mut *inner.subs.borrow_mut());
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
/// only when the value actually changes. A `Copy` handle, owned like a
/// [`Signal`].
pub struct Memo<T> {
    signal: Signal<T>,
}

impl<T> Clone for Memo<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Memo<T> {}

thread_local! {
    /// Effects of memos made outside any owner: they live as long as the app.
    static ROOT_EFFECTS: RefCell<Vec<Effect>> = const { RefCell::new(Vec::new()) };
}

/// Keeps `effect` running for as long as the current owner (or the app,
/// outside one).
pub(crate) fn keep_with_owner(effect: Effect) {
    OWNER.with_borrow(|o| match o {
        Some(o) => o.effects.borrow_mut().push(effect),
        None => ROOT_EFFECTS.with_borrow_mut(|r| r.push(effect)),
    });
}

impl<T: PartialEq + Clone + 'static> Memo<T> {
    /// A value computed by `f` from other signals.
    pub fn new(f: impl Fn() -> T + 'static) -> Self {
        let cell: Rc<Cell<Option<Signal<T>>>> = Rc::new(Cell::new(None));
        let c = cell.clone();
        let effect = Effect::new(move || {
            let v = f();
            match c.get() {
                None => c.set(Some(Signal::new(v))),
                Some(s) => s.set_if_changed(v),
            }
        });
        let signal = cell.get().expect("memo computed");
        keep_with_owner(effect);
        Memo { signal }
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
        let (s2, seen2) = (s, seen.clone());
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
        let (a2, b2) = (a, b);
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
        let (f2, x2, y2) = (flag, x, y);
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
        let n2 = n;
        let even = Memo::new(move || n2.get() % 2 == 0);
        let (runs, bump) = counter();
        let e2 = even;
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
        let (a1, a2) = (a, a);
        let b = Memo::new(move || a1.get() + 1);
        let c = Memo::new(move || a2.get() * 2);
        let (runs, bump) = counter();
        let (b2, c2) = (b, c);
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
        let (s2, d2) = (src, dst);
        let _copy = Effect::new(move || d2.set(s2.get() * 10));
        assert_eq!(dst.get_untracked(), 10);
        src.set(2);
        assert_eq!(dst.get_untracked(), 20);
    }

    #[test]
    fn untrack_and_set_if_changed() {
        let s = Signal::new(1);
        let (runs, bump) = counter();
        let s2 = s;
        let _e = Effect::new(move || {
            bump();
            untrack(|| s2.get());
        });
        s.set(2);
        assert_eq!(runs.get(), 1);
        let t = Signal::new(5);
        let t2 = t;
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
        let (k, (runs, bump)) = (keep, counter());
        let _kept = Effect::new(move || {
            bump();
            let _ = k.get();
        });
        for _ in 0..10_000 {
            let s2 = s;
            let e = Effect::new(move || {
                let _ = s2.get();
            });
            drop(e);
        }
        assert!(s.inner().subs.borrow().len() <= 2 * SWEEP_MIN, "{} entries", s.inner().subs.borrow().len());
        // Live readers survive sweeps and still run.
        let live: Vec<Effect> = (0..100)
            .map(|_| {
                let s2 = s;
                Effect::new(move || {
                    let _ = s2.get();
                })
            })
            .collect();
        for _ in 0..1_000 {
            let s2 = s;
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

    #[test]
    fn owners_free_their_signals() {
        let before = live_signals();
        let owner = Owner::new();
        let (a, m) = owner.with(|| {
            let a = Signal::new(1);
            (a, Memo::new(move || a.get() * 2))
        });
        let outside = Signal::new(0);
        assert_eq!(m.get(), 2);
        assert_eq!(live_signals(), before + 3);
        drop(owner);
        assert_eq!(live_signals(), before + 1);
        assert!(!a.is_alive());
        assert_eq!(a.try_get(), None);
        a.set(5); // a no-op once freed
        assert!(outside.is_alive());
        // Freed slots are reused, and the old handle stays dead.
        let b = Signal::new(7);
        assert!(!a.is_alive());
        assert_eq!(b.get(), 7);
    }

    #[test]
    fn effects_create_signals_in_their_owner() {
        let before = live_signals();
        let trigger = Signal::new(0);
        let made = Rc::new(Cell::new(0));
        let owner = Owner::new();
        let m = made.clone();
        let effect = owner.with(|| {
            Effect::new(move || {
                trigger.get();
                Signal::new(0u8);
                m.set(m.get() + 1);
            })
        });
        // Re-run outside the owner: the new signal still goes to it.
        trigger.set(1);
        assert_eq!(made.get(), 2);
        drop(effect);
        drop(owner);
        assert_eq!(live_signals(), before + 1); // only `trigger`
    }

    #[test]
    #[should_panic(expected = "freed")]
    fn reading_a_freed_signal_panics() {
        let owner = Owner::new();
        let s = owner.with(|| Signal::new(1));
        drop(owner);
        s.get();
    }
}
