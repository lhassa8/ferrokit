//! Thread-local state that stays reachable while the process exits.
//!
//! `std::process::exit` (and `NSApplication`'s `terminate:`, which calls it)
//! destroys the main thread's thread-locals one by one before the process
//! ends. A destructor that runs then (a window's views, an app's own
//! thread-local holding a signal) and reaches a thread-local already
//! destroyed panics, and a panic there aborts the app with a crash report.
//!
//! A [`UiCell`] has no destructor, so a thread-local holding one is never
//! destroyed and is always there to reach; what it holds goes when the
//! process does. Every Ferrokit thread-local holding more than plain values
//! is one. [`UiLocal`] gives its key the `RefCell` key's methods, so code
//! reads the same.

use std::cell::{Ref, RefCell, RefMut};
use std::mem::ManuallyDrop;
use std::thread::LocalKey;

/// A `RefCell` that never drops its value (see the module docs).
pub struct UiCell<T>(RefCell<ManuallyDrop<T>>);

impl<T> UiCell<T> {
    /// A cell holding `value`.
    pub const fn new(value: T) -> Self {
        Self(RefCell::new(ManuallyDrop::new(value)))
    }

    /// Borrows the value (panics if it's mutably borrowed).
    pub fn borrow(&self) -> Ref<'_, T> {
        Ref::map(self.0.borrow(), |v| &**v)
    }

    /// Borrows the value mutably (panics if it's borrowed).
    pub fn borrow_mut(&self) -> RefMut<'_, T> {
        RefMut::map(self.0.borrow_mut(), |v| &mut **v)
    }

    /// Borrows the value, or `Err` if it's mutably borrowed.
    pub fn try_borrow(&self) -> Result<Ref<'_, T>, std::cell::BorrowError> {
        self.0.try_borrow().map(|r| Ref::map(r, |v| &**v))
    }

    /// Borrows the value mutably, or `Err` if it's borrowed.
    pub fn try_borrow_mut(&self) -> Result<RefMut<'_, T>, std::cell::BorrowMutError> {
        self.0.try_borrow_mut().map(|r| RefMut::map(r, |v| &mut **v))
    }
}

/// `RefCell` keys' methods for a thread-local [`UiCell`].
pub trait UiLocal<T: 'static> {
    /// Calls `f` with the value borrowed.
    fn with_borrow<R>(&'static self, f: impl FnOnce(&T) -> R) -> R;
    /// Calls `f` with the value borrowed mutably.
    fn with_borrow_mut<R>(&'static self, f: impl FnOnce(&mut T) -> R) -> R;
    /// Sets the value (the old one is dropped).
    fn set(&'static self, value: T);
    /// Takes the value, leaving the default.
    fn take(&'static self) -> T
    where
        T: Default;
    /// Replaces the value, returning the old one.
    fn replace(&'static self, value: T) -> T;
}

impl<T: 'static> UiLocal<T> for LocalKey<UiCell<T>> {
    fn with_borrow<R>(&'static self, f: impl FnOnce(&T) -> R) -> R {
        self.with(|c| f(&c.borrow()))
    }

    fn with_borrow_mut<R>(&'static self, f: impl FnOnce(&mut T) -> R) -> R {
        self.with(|c| f(&mut c.borrow_mut()))
    }

    fn set(&'static self, value: T) {
        drop(self.replace(value));
    }

    fn take(&'static self) -> T
    where
        T: Default,
    {
        self.replace(T::default())
    }

    fn replace(&'static self, value: T) -> T {
        self.with(|c| std::mem::replace(&mut *c.borrow_mut(), value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_no_destructor() {
        assert!(!std::mem::needs_drop::<UiCell<Vec<String>>>());
    }

    /// A thread-local reached from another's destructor as the thread exits
    /// (`RefCell` here panics: "cannot access a Thread Local Storage value
    /// during or after destruction").
    #[test]
    fn reachable_while_the_thread_exits() {
        struct Reaches;
        impl Drop for Reaches {
            fn drop(&mut self) {
                COUNT.with_borrow_mut(|c| c.push(1));
            }
        }
        thread_local! {
            static HOLDER: RefCell<Option<Reaches>> = const { RefCell::new(None) };
            static COUNT: UiCell<Vec<u32>> = const { UiCell::new(Vec::new()) };
        }
        let exited = std::thread::spawn(|| {
            // HOLDER's destructor is registered first, so it runs last.
            HOLDER.with_borrow_mut(|_| ());
            COUNT.with_borrow_mut(|c| c.push(0));
            HOLDER.with_borrow_mut(|h| *h = Some(Reaches));
        })
        .join();
        assert!(exited.is_ok());
    }
}
