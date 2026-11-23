//! Async tasks on the main thread: [`spawn_local`] runs a future there, so
//! it can read and set signals between its awaits, and [`background`] and
//! [`sleep`] are futures it can await.
//!
//! The executor is the main run loop: a waker (called from any thread)
//! schedules the task's next poll as a run-loop block in the common modes,
//! so tasks go on while a menu is open or a window is being resized. Each
//! poll runs inside a [`batch`](crate::batch), with the task's owner
//! current. Ferrokit has no I/O runtime of its own: run I/O on your app's
//! runtime (tokio or another) and await its join handle from the task, or
//! use `on_runtime` (the `tokio` feature).
//!
//! AppKit's callback APIs have awaitable versions that return an
//! [`Answer`]: [`alert_async`](crate::alert_async),
//! [`open_panel_async`](crate::open_panel_async),
//! [`save_panel_async`](crate::save_panel_async) and
//! [`sheet_async`](crate::sheet_async).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Wake, Waker};
use std::time::Duration;

use ferrokit_core::kept::{UiCell, UiLocal};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::msg_send;

use crate::platform::{Native, Platform};
use crate::signal::{OwnerInner, batch, disown_task, own_task, untrack, with_weak_owner};

type LocalFuture = Pin<Box<dyn Future<Output = ()>>>;

struct Slot {
    /// `None` while it's being polled.
    future: Option<LocalFuture>,
    /// Woken during its own poll: poll again once that poll returns.
    repoll: bool,
    owner: Option<Weak<OwnerInner>>,
    waker: Arc<TaskWaker>,
}

thread_local! {
    static TASKS: UiCell<HashMap<u64, Slot>> = UiCell::new(HashMap::new());
    static NEXT: Cell<u64> = const { Cell::new(0) };
    /// The task being polled now.
    static CURRENT: Cell<Option<u64>> = const { Cell::new(None) };
}

/// Wakes a task from any thread: its next poll is posted to the main run
/// loop, once however many times it's woken before that poll.
struct TaskWaker {
    id: u64,
    scheduled: AtomicBool,
}

impl Wake for TaskWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        if !self.scheduled.swap(true, Ordering::AcqRel) {
            let id = self.id;
            Native::on_main(Box::new(move || poll(id)));
        }
    }
}

/// A task started by [`spawn_local`]. A `Copy` handle; dropping it neither
/// cancels nor detaches the task.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Task(u64);

impl fmt::Debug for Task {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Task").field("id", &self.0).field("finished", &self.is_finished()).finish()
    }
}

impl Task {
    /// Stops the task: its future is dropped now (at the `.await` it's
    /// waiting at; called from inside the task, as soon as its current
    /// poll returns). Work a [`background`] call already started on its
    /// thread runs to the end; its result is dropped. Nothing happens if
    /// the task has finished.
    pub fn cancel(self) {
        cancel_all([self.0]);
    }

    /// Keeps the task running after its owner goes (a closed window, a
    /// removed view), until it finishes or is cancelled.
    pub fn detach(self) {
        let owner = TASKS.with_borrow_mut(|t| t.get_mut(&self.0).and_then(|s| s.owner.take()));
        if let Some(owner) = owner {
            disown_task(&owner, self.0);
        }
    }

    /// Whether the task is over: its future finished, panicked or was
    /// cancelled (also by its owner going).
    pub fn is_finished(self) -> bool {
        !TASKS.with_borrow(|t| t.contains_key(&self.0))
    }
}

/// Runs `future` on the main thread, as a task of the main run loop: it can
/// read and set signals and touch views between its `.await`s, so it needn't
/// be `Send`. Call from the main thread. The first poll is on a later pass
/// of the run loop, not inside this call.
///
/// The task belongs to the current owner, like the signals made now: when
/// the owner goes (its window closes, its view is removed) the future is
/// dropped and the task stops at the `.await` it's waiting at
/// ([`Task::detach`] keeps it going). Outside any owner (in
/// [`App::on_launch`](crate::App::on_launch) or a [`later`](crate::later)
/// callback) it runs until it finishes. A panic in it is reported like any other failure (see
/// [`App::on_error`](crate::App::on_error), as "an async task") and ends
/// the task; the app goes on.
///
/// Await [`background`] for blocking work, [`sleep`] for a pause, and your
/// runtime's join handle for async I/O (reqwest, sqlx: they need their own
/// runtime, which Ferrokit doesn't provide). Don't block in a task: a poll
/// runs on the main thread, between events.
///
/// ```no_run
/// use ferrokit::*;
///
/// let status = Signal::new(String::new());
/// let count = button("Count words", move || {
///     spawn_local(async move {
///         status.set("Counting…".into());
///         let words = background(|| std::fs::read_to_string("/usr/share/dict/words").map(|t| t.lines().count())).await;
///         match words {
///             Ok(n) => status.set(format!("{n} words")),
///             Err(e) => status.set(format!("Couldn't read the list: {e}")),
///         }
///     });
/// });
/// # let _ = count;
/// ```
pub fn spawn_local(future: impl Future<Output = ()> + 'static) -> Task {
    let _ = crate::mtm();
    let id = NEXT.with(|n| n.replace(n.get() + 1));
    let owner = own_task(id);
    let waker = Arc::new(TaskWaker { id, scheduled: AtomicBool::new(false) });
    TASKS.with_borrow_mut(|t| t.insert(id, Slot { future: Some(Box::pin(future)), repoll: false, owner, waker: waker.clone() }));
    waker.wake_by_ref();
    Task(id)
}

/// Cancels the tasks `ids` (an owner going, or [`Task::cancel`]). Their
/// futures are dropped outside the task table's borrow: dropping one may
/// close a window, whose owner cancels its own tasks.
pub(crate) fn cancel_all(ids: impl IntoIterator<Item = u64>) {
    let mut gone = Vec::new();
    for id in ids {
        let slot = TASKS.try_with(|t| t.borrow_mut().remove(&id)).ok().flatten();
        if let Some(slot) = slot {
            if let Some(owner) = &slot.owner {
                disown_task(owner, id);
            }
            gone.push(slot);
        }
    }
    drop(gone);
}

/// Polls task `id` once, if it's still there and not being polled already.
fn poll(id: u64) {
    let taken = TASKS.with_borrow_mut(|t| {
        let slot = t.get_mut(&id)?;
        match slot.future.take() {
            Some(f) => Some((f, slot.waker.clone(), slot.owner.clone())),
            None => {
                // Woken while it's being polled (further out, in a nested
                // run loop): poll again once that poll returns. This
                // block is no longer pending, so the next wake posts one.
                slot.repoll = true;
                slot.waker.scheduled.store(false, Ordering::Release);
                None
            }
        }
    });
    let Some((mut future, waker, owner)) = taken else { return };
    // Woken from here on means poll again.
    waker.scheduled.store(false, Ordering::Release);
    let task_waker = Waker::from(waker.clone());
    let prev = CURRENT.replace(Some(id));
    let polled = crate::failure::guard("an async task", None, || {
        Some(untrack(|| with_weak_owner(owner.as_ref(), || batch(|| future.as_mut().poll(&mut Context::from_waker(&task_waker))))))
    });
    CURRENT.set(prev);
    match polled {
        Some(Poll::Pending) => {
            // Put back unless it was cancelled during its own poll; then it
            // is dropped here, outside the table's borrow.
            let mut future = Some(future);
            let again = TASKS.with_borrow_mut(|t| match t.get_mut(&id) {
                Some(slot) => {
                    slot.future = future.take();
                    std::mem::take(&mut slot.repoll)
                }
                None => false,
            });
            drop(future);
            if again {
                waker.wake_by_ref();
            }
        }
        // Finished, or panicked (reported by the guard): the task is over.
        Some(Poll::Ready(())) | None => {
            drop(future);
            cancel_all([id]);
        }
    }
}

/// The task being polled now, if any (for [`background`]).
fn current() -> Option<u64> {
    CURRENT.with(Cell::get)
}

struct Shared<T> {
    result: RefCell<Option<T>>,
    waker: RefCell<Option<Waker>>,
    /// The task that awaited it: ended if the work panics.
    task: Cell<Option<u64>>,
}

/// A future for work on a background thread, from [`background`].
#[must_use = "the result is dropped unless it's awaited"]
pub struct Background<T> {
    shared: Rc<Shared<T>>,
}

impl<T> fmt::Debug for Background<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Background").field("done", &self.shared.result.borrow().is_some()).finish()
    }
}

/// Runs `work` on a background thread now, and returns a future for its
/// result, to await in a [`spawn_local`] task: [`spawn`](crate::spawn) for
/// async code. Call from the main thread.
///
/// If `work` panics, the panic is reported (as "a background task") and
/// the task awaiting it ends there, as `spawn`'s continuation doesn't run.
/// Return a `Result` for errors you expect. Dropping the future (the task
/// was cancelled) doesn't stop `work`; its result is dropped.
pub fn background<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Background<T> {
    let shared = Rc::new(Shared { result: RefCell::new(None), waker: RefCell::new(None), task: Cell::new(None) });
    let weak = Rc::downgrade(&shared);
    crate::spawn_outcome(work, move |outcome| {
        let Some(shared) = weak.upgrade() else { return }; // nobody's waiting
        match outcome {
            Ok(result) => {
                *shared.result.borrow_mut() = Some(result);
                let waker = shared.waker.borrow_mut().take();
                if let Some(w) = waker {
                    w.wake();
                }
            }
            // The panic is reported already; the awaiting task ends.
            Err(_) => {
                if let Some(task) = shared.task.get() {
                    // On a later pass: this continuation may run inside
                    // that task's poll (a nested run loop).
                    crate::later(move || Task(task).cancel());
                }
            }
        }
    });
    Background { shared }
}

impl<T> Future for Background<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        if let Some(result) = self.shared.result.borrow_mut().take() {
            return Poll::Ready(result);
        }
        if self.shared.task.get().is_none() {
            self.shared.task.set(current());
        }
        *self.shared.waker.borrow_mut() = Some(cx.waker().clone());
        Poll::Pending
    }
}

struct Timer {
    fired: Cell<bool>,
    waker: RefCell<Option<Waker>>,
}

/// A future that's ready after a time, from [`sleep`].
#[must_use = "a sleep does nothing unless it's awaited"]
pub struct Sleep {
    duration: Duration,
    state: Rc<Timer>,
    timer: Option<Retained<AnyObject>>,
}

impl fmt::Debug for Sleep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sleep").field("duration", &self.duration).field("fired", &self.state.fired.get()).finish()
    }
}

/// A future that's ready after `duration`, timed on the main run loop (in
/// the common modes, so it fires while a menu is open). The timer starts at
/// the first poll. Call from the main thread; await it in a
/// [`spawn_local`] task.
///
/// ```no_run
/// use ferrokit::*;
/// use std::time::Duration;
///
/// let saved = Signal::new(false);
/// spawn_local(async move {
///     saved.set(true);
///     sleep(Duration::from_secs(2)).await;
///     saved.set(false); // "Saved" shows for two seconds
/// });
/// ```
pub fn sleep(duration: Duration) -> Sleep {
    let _ = crate::mtm();
    Sleep { duration, state: Rc::new(Timer { fired: Cell::new(false), waker: RefCell::new(None) }), timer: None }
}

impl Future for Sleep {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.state.fired.get() {
            return Poll::Ready(());
        }
        *self.state.waker.borrow_mut() = Some(cx.waker().clone());
        if self.timer.is_none() {
            let state = self.state.clone();
            let block = block2::RcBlock::new(move |_timer: std::ptr::NonNull<AnyObject>| {
                state.fired.set(true);
                let waker = state.waker.borrow_mut().take();
                if let Some(w) = waker {
                    w.wake();
                }
            });
            let seconds = self.duration.as_secs_f64();
            // SAFETY: NSTimer's block-based constructor (macOS 10.12), added
            // to the main run loop in the common modes; it keeps the block.
            let timer: Retained<AnyObject> = unsafe {
                let timer: Retained<AnyObject> = msg_send![objc2::class!(NSTimer), timerWithTimeInterval: seconds, repeats: false, block: &*block];
                let run_loop: Retained<AnyObject> = msg_send![objc2::class!(NSRunLoop), mainRunLoop];
                let () = msg_send![&*run_loop, addTimer: &*timer, forMode: objc2_foundation::NSRunLoopCommonModes];
                timer
            };
            self.timer = Some(timer);
        }
        Poll::Pending
    }
}

impl Drop for Sleep {
    fn drop(&mut self) {
        if let Some(timer) = self.timer.take() {
            // SAFETY: NSTimer's invalidate (it releases the block), on the
            // main thread where it was scheduled.
            unsafe {
                let () = msg_send![&*timer, invalidate];
            }
        }
    }
}

struct AnswerState<T> {
    value: RefCell<Option<T>>,
    /// Answered (the value may have been taken by the poll since).
    answered: Cell<bool>,
    waker: RefCell<Option<Waker>>,
}

/// A future for an answer that comes later, from a dialog or a sheet:
/// [`alert_async`](crate::alert_async),
/// [`open_panel_async`](crate::open_panel_async),
/// [`save_panel_async`](crate::save_panel_async),
/// [`sheet_async`](crate::sheet_async). Await it in a [`spawn_local`]
/// task.
///
/// Dropping it before the answer comes (the task was cancelled, or its
/// owner went) closes the dialog it waits for, as Cancel would.
#[must_use = "the dialog's answer is dropped unless it's awaited"]
pub struct Answer<T> {
    state: Rc<AnswerState<T>>,
    on_drop: Option<Box<dyn FnOnce()>>,
}

impl<T> fmt::Debug for Answer<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Answer").field("answered", &self.state.answered.get()).finish()
    }
}

/// The sending side of an [`Answer`].
pub(crate) struct Responder<T>(Rc<AnswerState<T>>);

impl<T> Responder<T> {
    /// Gives the answer (the first one counts) and wakes the task awaiting it.
    pub(crate) fn send(&self, value: T) {
        if self.0.answered.replace(true) {
            return;
        }
        *self.0.value.borrow_mut() = Some(value);
        let waker = self.0.waker.borrow_mut().take();
        if let Some(w) = waker {
            w.wake();
        }
    }
}

/// A new answer and its sender; `on_drop` runs if the answer is dropped
/// before it comes (to close the dialog).
pub(crate) fn answer<T>(on_drop: impl FnOnce() + 'static) -> (Answer<T>, Responder<T>) {
    let state = Rc::new(AnswerState { value: RefCell::new(None), answered: Cell::new(false), waker: RefCell::new(None) });
    (Answer { state: state.clone(), on_drop: Some(Box::new(on_drop)) }, Responder(state))
}

impl<T> Future for Answer<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        if let Some(value) = self.state.value.borrow_mut().take() {
            return Poll::Ready(value);
        }
        *self.state.waker.borrow_mut() = Some(cx.waker().clone());
        Poll::Pending
    }
}

impl<T> Drop for Answer<T> {
    fn drop(&mut self) {
        if !self.state.answered.get()
            && let Some(close) = self.on_drop.take()
        {
            // Closing the dialog answers it (into this dropped answer).
            close();
        }
    }
}

/// A future for a task on a tokio runtime, from [`on_runtime`].
#[cfg(feature = "tokio")]
#[must_use = "the result is dropped unless it's awaited (the runtime task still runs until then)"]
pub struct OnRuntime<T> {
    join: tokio::task::JoinHandle<T>,
}

#[cfg(feature = "tokio")]
impl<T> fmt::Debug for OnRuntime<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OnRuntime").field("finished", &self.join.is_finished()).finish()
    }
}

/// Runs `future` on the tokio runtime `handle` (your app's own, for I/O
/// such as reqwest or sqlx), and returns a future for its output, to await
/// in a [`spawn_local`] task. Needs the `tokio` feature.
///
/// Unlike awaiting tokio's `JoinHandle` directly:
/// - **Cancelling goes both ways:** when the awaiting task is cancelled
///   (its window closed, [`Task::cancel`]), the runtime task is aborted at
///   its next `.await`.
/// - **No `JoinError` to unwrap:** a panic in the runtime task is reported
///   like one in the awaiting task (see
///   [`App::on_error`](crate::App::on_error)), and ends that task. So is the
///   runtime task being cancelled from outside (its runtime shut down).
///
/// ```no_run
/// use ferrokit::*;
///
/// let runtime = tokio::runtime::Runtime::new().unwrap(); // kept for the app's life
/// let handle = runtime.handle().clone();
/// let status = Signal::new(String::new());
/// spawn_local(async move {
///     status.set("Loading…".into());
///     let n = on_runtime(&handle, async {
///         // reqwest, sqlx, tokio::fs… run here, on the runtime
///         42
///     })
///     .await;
///     status.set(format!("{n} items"));
/// });
/// # std::mem::forget(runtime);
/// ```
#[cfg(feature = "tokio")]
pub fn on_runtime<F>(handle: &tokio::runtime::Handle, future: F) -> OnRuntime<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    OnRuntime { join: handle.spawn(future) }
}

#[cfg(feature = "tokio")]
impl<T> Future for OnRuntime<T> {
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        match Pin::new(&mut self.join).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(value)) => Poll::Ready(value),
            // Carried into the awaiting task, which reports it and ends.
            Poll::Ready(Err(e)) if e.is_panic() => std::panic::resume_unwind(e.into_panic()),
            Poll::Ready(Err(_)) => panic!("its task on the tokio runtime was cancelled (did the runtime shut down?)"),
        }
    }
}

#[cfg(feature = "tokio")]
impl<T> Drop for OnRuntime<T> {
    fn drop(&mut self) {
        self.join.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Polls `f` to the end on this thread, parking between polls.
    fn block_on<F: Future>(f: F) -> F::Output {
        struct Unpark(std::thread::Thread);
        impl Wake for Unpark {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
        }
        let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
        let mut f = std::pin::pin!(f);
        loop {
            if let Poll::Ready(v) = f.as_mut().poll(&mut Context::from_waker(&waker)) {
                return v;
            }
            std::thread::park_timeout(Duration::from_millis(50));
        }
    }

    #[test]
    fn an_answer_is_ready_once_sent_and_the_first_answer_counts() {
        let closed = Rc::new(Cell::new(false));
        let c = closed.clone();
        let (answer, responder) = answer::<u32>(move || c.set(true));
        responder.send(7);
        responder.send(8);
        assert_eq!(block_on(answer), 7);
        assert!(!closed.get(), "an answered dialog isn't closed again");
    }

    #[test]
    fn dropping_an_unanswered_answer_closes_its_dialog() {
        let closed = Rc::new(Cell::new(false));
        let c = closed.clone();
        let (answer, responder) = answer::<u32>(move || c.set(true));
        drop(answer);
        assert!(closed.get());
        responder.send(1); // the dialog's late answer goes nowhere
    }

    #[cfg(feature = "tokio")]
    #[test]
    fn on_runtime_gives_the_output_and_carries_a_panic() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        assert_eq!(block_on(on_runtime(rt.handle(), async { 6 * 7 })), 42);
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| block_on(on_runtime(rt.handle(), async { panic!("boom") }))));
        let payload = caught.expect_err("the panic reaches the awaiting side");
        assert_eq!(payload.downcast_ref::<&str>(), Some(&"boom"));
    }

    #[cfg(feature = "tokio")]
    #[test]
    fn dropping_on_runtime_aborts_the_runtime_task() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let finished = Arc::new(AtomicBool::new(false));
        let f = finished.clone();
        let started = Arc::new(AtomicBool::new(false));
        let s = started.clone();
        let fut = on_runtime(rt.handle(), async move {
            s.store(true, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(300)).await;
            f.store(true, Ordering::SeqCst);
        });
        while !started.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
        drop(fut);
        std::thread::sleep(Duration::from_millis(500));
        assert!(!finished.load(Ordering::SeqCst), "the runtime task was aborted");
    }
}
