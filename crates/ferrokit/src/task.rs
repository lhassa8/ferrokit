//! Async tasks on the main thread: [`spawn_local`] runs a future there, so
//! it can read and set signals between its awaits, and [`background`] and
//! [`sleep`] are futures it can await.
//!
//! The executor is the main run loop: a waker (called from any thread)
//! schedules the task's next poll as a run-loop block in the common modes,
//! so tasks go on while a menu is open or a window is being resized. Each
//! poll runs inside a [`batch`](crate::batch), with the task's owner
//! current. Ferrokit has no I/O runtime of its own: run I/O on your app's
//! runtime (tokio or another) and await its join handle from the task.

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
/// ([`Task::detach`] keeps it going). Outside any owner it runs until it
/// finishes. A panic in it is reported like any other failure (see
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
