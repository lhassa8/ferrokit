//! Where AppKit calls into this crate (a table's data source, a text
//! view's delegate): `ferrokit` installs its failure guard here, so a
//! panic or an Objective-C exception in such a call, or in the app's code
//! it runs (a column's value, a highlighter), is caught and reported there
//! and never unwinds into AppKit.

use std::cell::Cell;

/// Runs the closure it is given, catching what fails in it; the place is
/// what was running, for the report.
pub type Guard = fn(&'static str, &mut dyn FnMut());

thread_local! {
    /// The main thread's guard (AppKit calls in there).
    static GUARD: Cell<Option<Guard>> = const { Cell::new(None) };
}

/// Installs the guard for this thread's calls.
pub fn set_guard(guard: Guard) {
    GUARD.with(|g| g.set(Some(guard)));
}

/// Runs `f` under the installed guard: its result, or `fallback` if it
/// failed. With no guard installed, `f` just runs.
pub fn guarded<R>(place: &'static str, fallback: R, f: impl FnOnce() -> R) -> R {
    let Some(guard) = GUARD.try_with(Cell::get).ok().flatten() else { return f() };
    let (mut f, mut out) = (Some(f), None);
    guard(place, &mut || {
        if let Some(f) = f.take() {
            out = Some(f());
        }
    });
    out.unwrap_or(fallback)
}
