//! Values a view shows: fixed, or computed from signals and kept current.
//!
//! Wherever a view or modifier takes `impl IntoValue<T>`, it takes either a
//! plain value or a closure that reads signals:
//!
//! ```
//! use ferrokit::*;
//!
//! let done = Signal::new(false);
//! let fixed = text("Packing list").color(Color::Secondary);
//! let live = text(move || if done.get() { "All packed".to_string() } else { "Still packing".to_string() })
//!     .color(move || if done.get() { Color::Green } else { Color::Primary });
//! # let _ = (fixed, live);
//! ```
//!
//! A closure runs again whenever a signal it read changes, and the view
//! updates in place, as SwiftUI's views follow their state.

use std::rc::Rc;

/// A value a view shows: fixed, or computed from signals (see the module
/// docs). Made by [`IntoValue`].
pub enum Value<T> {
    /// The same value always.
    Fixed(T),
    /// Computed again when the signals it reads change.
    Computed(Rc<dyn Fn() -> T>),
}

impl<T: Clone> Clone for Value<T> {
    fn clone(&self) -> Self {
        match self {
            Value::Fixed(v) => Value::Fixed(v.clone()),
            Value::Computed(f) => Value::Computed(f.clone()),
        }
    }
}

impl<T: Clone> Value<T> {
    /// The value now (reading its signals, inside an effect, subscribes it).
    pub fn get(&self) -> T {
        match self {
            Value::Fixed(v) => v.clone(),
            Value::Computed(f) => f(),
        }
    }

    /// Whether it never changes.
    pub fn is_fixed(&self) -> bool {
        matches!(self, Value::Fixed(_))
    }
}

/// What a view or modifier taking a value accepts: the value itself, or a
/// closure computing it from signals.
pub trait IntoValue<T> {
    /// The [`Value`].
    fn into_value(self) -> Value<T>;
}

impl<T, F: Fn() -> T + 'static> IntoValue<T> for F {
    fn into_value(self) -> Value<T> {
        Value::Computed(Rc::new(self))
    }
}

macro_rules! fixed {
    ($($t:ty),*) => {$(
        impl IntoValue<$t> for $t {
            fn into_value(self) -> Value<$t> {
                Value::Fixed(self)
            }
        }
    )*};
}

fixed!(crate::Color, f64, bool, String);

impl IntoValue<String> for &str {
    fn into_value(self) -> Value<String> {
        Value::Fixed(self.to_string())
    }
}

impl IntoValue<String> for &String {
    fn into_value(self) -> Value<String> {
        Value::Fixed(self.clone())
    }
}
