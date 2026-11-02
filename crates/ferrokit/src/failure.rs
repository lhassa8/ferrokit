//! Failures at the boundary: an Objective-C exception or a Rust panic in
//! code AppKit calls into Ferrokit for (an action, a menu item, a timer,
//! a canvas event, a notification, a sheet's completion) is caught where
//! it entered, reported with where it happened, and the app goes on. Left
//! alone, AppKit would swallow an exception without a word, leaving the
//! rest of that work undone.

use ferrokit_core::kept::{UiCell, UiLocal};
use std::panic::AssertUnwindSafe;
use std::rc::Rc;

/// Something that failed in the app's code or in AppKit, caught where
/// AppKit called in (see [`App::on_error`](crate::App::on_error)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// What was running: "a control's action", "a timer"…
    pub place: &'static str,
    /// What happened: the exception's name and reason, or the panic's message.
    pub message: String,
    /// An Objective-C exception (`true`) or a Rust panic.
    pub exception: bool,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let what = if self.exception { "an Objective-C exception" } else { "a panic" };
        write!(f, "{what} in {}: {}", self.place, self.message)
    }
}

type Hook = Rc<dyn Fn(&Failure)>;

thread_local! {
    /// The failures so far (the last 100), for checks.
    static FAILURES: UiCell<Vec<Failure>> = const { UiCell::new(Vec::new()) };
    static HOOK: UiCell<Option<Hook>> = const { UiCell::new(None) };
}

/// Runs `f`, catching an exception or a panic in it: reported, and
/// `fallback` returned instead.
pub(crate) fn guard<R>(place: &'static str, fallback: R, f: impl FnOnce() -> R) -> R {
    // The exception catcher sits next to the code (an Objective-C exception
    // must never reach `catch_unwind`: Rust aborts on one); panics pass
    // through it to `catch_unwind`.
    let caught = std::panic::catch_unwind(AssertUnwindSafe(|| objc2::exception::catch(AssertUnwindSafe(f))));
    let failure = match caught {
        Ok(Ok(r)) => return r,
        Err(payload) => {
            let message = payload.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| payload.downcast_ref::<String>().cloned()).unwrap_or_else(|| "(no message)".into());
            Failure { place, message, exception: false }
        }
        Ok(Err(exception)) => {
            let message = exception.as_deref().map_or_else(|| "(nil)".into(), |e| {
                // SAFETY: an NSException's name and reason.
                let (name, reason): (Option<objc2::rc::Retained<objc2_foundation::NSString>>, Option<objc2::rc::Retained<objc2_foundation::NSString>>) =
                    unsafe { (objc2::msg_send![e, name], objc2::msg_send![e, reason]) };
                format!("{}: {}", name.map(|n| n.to_string()).unwrap_or_default(), reason.map(|r| r.to_string()).unwrap_or_default())
            });
            Failure { place, message, exception: true }
        }
    };
    report(failure);
    fallback
}

/// [`guard`] around a [`batch`](crate::batch): the usual way in from AppKit.
pub(crate) fn entry<R>(place: &'static str, fallback: R, f: impl FnOnce() -> R) -> R {
    guard(place, fallback, || crate::signal::batch(f))
}

fn report(failure: Failure) {
    eprintln!("ferrokit: {failure} (the app goes on)");
    let _ = FAILURES.try_with(|f| {
        let mut f = f.borrow_mut();
        if f.len() >= 100 {
            f.remove(0);
        }
        f.push(failure.clone());
    });
    let hook = HOOK.try_with(|h| h.borrow().clone()).ok().flatten();
    if let Some(hook) = hook {
        // A hook that fails itself is only printed.
        let _ = std::panic::catch_unwind(AssertUnwindSafe(|| hook(&failure)));
    }
}

pub(crate) fn set_hook(f: impl Fn(&Failure) + 'static) {
    HOOK.with_borrow_mut(|h| *h = Some(Rc::new(f)));
}

/// The failures caught so far (the last 100).
pub(crate) fn caught() -> Vec<Failure> {
    FAILURES.with_borrow(Clone::clone)
}
