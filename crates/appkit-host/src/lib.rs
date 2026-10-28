//! Places real AppKit views inside a window whose content is drawn by a GPU
//! host framework (Iced first). Host-agnostic: it only needs the window's
//! `raw-window-handle` and layout rectangles in the host's logical points.
//!
//! Main thread only. Every function panics off the main thread.

mod geometry;
mod host;
pub mod sync;

pub use geometry::{Frames, Rect, frames};
pub use host::{AttachError, ViewId, attach, content_view, hide, is_attached, mount, place, scale, set_suppressed, unmount};
