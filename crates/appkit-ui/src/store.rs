//! Signals kept in the app's user defaults: settings that survive a relaunch.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2_foundation::{NSString, NSUserDefaults};

use crate::signal::{Effect, Signal, untrack};

/// A value [`stored`] can keep in user defaults.
pub trait Stored: Clone + PartialEq + 'static {
    /// Reads the value under `key`, if there is one.
    fn read(defaults: &NSUserDefaults, key: &NSString) -> Option<Self>;
    /// Writes the value under `key`.
    fn write(&self, defaults: &NSUserDefaults, key: &NSString);
}

fn present(defaults: &NSUserDefaults, key: &NSString) -> bool {
    defaults.objectForKey(key).is_some()
}

impl Stored for bool {
    fn read(d: &NSUserDefaults, key: &NSString) -> Option<Self> {
        present(d, key).then(|| d.boolForKey(key))
    }
    fn write(&self, d: &NSUserDefaults, key: &NSString) {
        d.setBool_forKey(*self, key);
    }
}

impl Stored for f64 {
    fn read(d: &NSUserDefaults, key: &NSString) -> Option<Self> {
        present(d, key).then(|| d.doubleForKey(key))
    }
    fn write(&self, d: &NSUserDefaults, key: &NSString) {
        d.setDouble_forKey(*self, key);
    }
}

impl Stored for i64 {
    fn read(d: &NSUserDefaults, key: &NSString) -> Option<Self> {
        present(d, key).then(|| d.integerForKey(key) as i64)
    }
    fn write(&self, d: &NSUserDefaults, key: &NSString) {
        d.setInteger_forKey(*self as isize, key);
    }
}

impl Stored for String {
    fn read(d: &NSUserDefaults, key: &NSString) -> Option<Self> {
        d.stringForKey(key).map(|s| s.to_string())
    }
    fn write(&self, d: &NSUserDefaults, key: &NSString) {
        // SAFETY: an NSString is a property-list value.
        unsafe { d.setObject_forKey(Some(&NSString::from_str(self)), key) };
    }
}

thread_local! {
    /// The write-back effects of stored signals (they live as long as the app).
    static WRITERS: RefCell<Vec<Effect>> = const { RefCell::new(Vec::new()) };
}

/// A signal holding the user-defaults value under `key` (or `default` when
/// there is none yet); every change is written back. Bind it to a control
/// in a Settings window and read it anywhere.
pub fn stored<T: Stored>(key: &str, default: T) -> Signal<T> {
    let defaults = NSUserDefaults::standardUserDefaults();
    let key: Retained<NSString> = NSString::from_str(key);
    let signal = Signal::new(T::read(&defaults, &key).unwrap_or(default));
    let s = signal;
    let mut first = true;
    let writer = Effect::new(move || {
        let value = s.get();
        // Nothing to write for the initial value: an untouched setting keeps
        // following the default.
        if !std::mem::take(&mut first) {
            untrack(|| value.write(&defaults, &key));
        }
    });
    WRITERS.with_borrow_mut(|w| w.push(writer));
    signal
}
