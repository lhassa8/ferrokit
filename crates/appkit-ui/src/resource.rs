//! Async resources: a value loaded off the main thread from the signals it
//! reads, with loading and error states.

use std::cell::Cell;
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::signal::{Effect, Signal, keep_with_owner, untrack};

/// Tells a load whether its result is still wanted. A load is cancelled when
/// a newer one starts (its inputs changed, or [`Resource::refetch`]) or the
/// resource's owner goes; long work can check and stop early.
#[derive(Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    /// Whether a newer load replaced this one (its result will be dropped).
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

impl fmt::Debug for Cancel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Cancel").field(&self.is_cancelled()).finish()
    }
}

/// A resource's state, from [`Resource::state`].
#[derive(Clone, Debug, PartialEq)]
pub enum Load<T> {
    /// The first load hasn't finished.
    Loading,
    /// The latest finished load succeeded (a newer one may be running: see
    /// [`Resource::loading`]).
    Ready(T),
    /// The latest finished load failed, with its error's text.
    Failed(String),
}

/// A value loaded on a background thread from inputs read on the main
/// thread. When the signals `source` reads change, a new load starts and the
/// one running is cancelled: only the newest result is ever shown, so
/// search-as-you-type can't show an older query's rows. The last value stays
/// readable while a new one loads.
///
/// A `Copy` handle, owned like a [`Signal`]: when its owner goes, the
/// running load is cancelled and its result dropped.
pub struct Resource<T> {
    value: Signal<Option<T>>,
    error: Signal<Option<String>>,
    loading: Signal<bool>,
    reload: Signal<u64>,
}

impl<T> Clone for Resource<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Resource<T> {}

/// Cancels the running load when the resource's effect goes.
struct CancelOnDrop(Rc<Cell<Option<Cancel>>>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(c) = self.0.take() {
            c.cancel();
        }
    }
}

impl<T: Clone + Send + 'static> Resource<T> {
    /// Loads with `fetch(source())` on a background thread, again whenever
    /// the signals `source` reads change. Call from the main thread.
    ///
    /// ```no_run
    /// use appkit_ui::*;
    ///
    /// let query = Signal::new(String::new());
    /// let matches = Resource::new(move || query.get(), |q: String, cancel: &Cancel| {
    ///     let words = std::fs::read_to_string("/usr/share/dict/words").map_err(|e| e.to_string())?;
    ///     let mut found = Vec::new();
    ///     for w in words.lines().filter(|w| w.starts_with(&q)) {
    ///         if cancel.is_cancelled() {
    ///             break; // a newer query started; this result won't be shown
    ///         }
    ///         found.push(w.to_string());
    ///     }
    ///     Ok::<_, String>(found)
    /// });
    /// let status = label(move || match matches.state() {
    ///     Load::Loading => "Searching…".into(),
    ///     Load::Ready(words) => format!("{} words", words.len()),
    ///     Load::Failed(e) => format!("Couldn't search: {e}"),
    /// });
    /// # let _ = status;
    /// ```
    pub fn new<S, E>(source: impl Fn() -> S + 'static, fetch: impl Fn(S, &Cancel) -> Result<T, E> + Send + Sync + 'static) -> Self
    where
        S: Send + 'static,
        E: fmt::Display,
    {
        let resource = Resource { value: Signal::new(None), error: Signal::new(None), loading: Signal::new(true), reload: Signal::new(0) };
        let Resource { value, error, loading, reload } = resource;
        let fetch = Arc::new(fetch);
        let running: Rc<Cell<Option<Cancel>>> = Rc::default();
        let guard = CancelOnDrop(running.clone());
        let effect = Effect::new(move || {
            let _ = &guard;
            reload.get();
            let input = source();
            untrack(|| {
                let cancel = Cancel::default();
                if let Some(old) = running.replace(Some(cancel.clone())) {
                    old.cancel();
                }
                loading.set_if_changed(true);
                let (fetch, theirs) = (fetch.clone(), cancel.clone());
                crate::spawn(
                    move || fetch(input, &theirs).map_err(|e| e.to_string()),
                    move |result| {
                        if cancel.is_cancelled() {
                            return; // a newer load is running (or the owner went)
                        }
                        match result {
                            Ok(v) => {
                                value.set(Some(v));
                                error.set_if_changed(None);
                            }
                            Err(e) => error.set(Some(e)),
                        }
                        loading.set_if_changed(false);
                    },
                );
            });
        });
        keep_with_owner(effect);
        resource
    }
}

impl<T: Clone + 'static> Resource<T> {
    /// The latest loaded value (kept while a newer load runs), subscribing
    /// the running effect. `None` until the first load succeeds.
    pub fn get(&self) -> Option<T> {
        self.value.get()
    }

    /// The latest value, without subscribing (for event handlers).
    pub fn get_untracked(&self) -> Option<T> {
        self.value.get_untracked()
    }

    /// Reads the latest value through `f` without copying it.
    pub fn with<R>(&self, f: impl FnOnce(Option<&T>) -> R) -> R {
        self.value.with(|v| f(v.as_ref()))
    }

    /// Whether a load is running.
    pub fn loading(&self) -> bool {
        self.loading.get()
    }

    /// The error of the latest finished load, if it failed.
    pub fn error(&self) -> Option<String> {
        self.error.get()
    }

    /// Loading, the latest value, or the latest error: whichever the most
    /// recent finished load produced (`Loading` only before the first).
    pub fn state(&self) -> Load<T> {
        match (self.error.get(), self.value.get()) {
            (Some(e), _) => Load::Failed(e),
            (None, Some(v)) => Load::Ready(v),
            (None, None) => Load::Loading,
        }
    }

    /// Loads again with the same inputs (the running load is cancelled).
    pub fn refetch(&self) {
        self.reload.update(|n| *n += 1);
    }
}
