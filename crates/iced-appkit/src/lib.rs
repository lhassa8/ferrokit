//! Real AppKit views inside Iced 0.14 windows on macOS.
//!
//! Phase 1, milestone M0: the embedding core only (attach and `NativeSlot`).

mod slot;

pub use appkit_host::{self as host, Rect, ViewId};
pub use slot::NativeSlot;

/// Attaches to an Iced window. Run the returned task once the window exists
/// (e.g. from the task that opens it).
pub fn attach<T: Send + 'static>(window: iced::window::Id, then: impl FnOnce(bool) -> T + Send + 'static) -> iced::Task<T> {
    iced::window::run(window, move |w| then(appkit_host::attach(&w).is_ok()))
}
