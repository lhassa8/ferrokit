//! Places real AppKit views inside a window whose content is drawn by a GPU
//! host framework (Iced first). Host-agnostic: it only needs the window's
//! `raw-window-handle` and layout rectangles in the host's logical points.
//!
//! Main thread only. Every function panics off the main thread.

#![warn(missing_docs)]

mod appearance;
mod events;
mod geometry;
mod host;
mod keys;
mod snapshot;
pub mod sync;
pub mod table;
pub mod text;
pub mod testing;

pub use appearance::{Rgba, SystemColors, generation as appearance_generation, is_dark, system_colors};
pub use events::{Event, request_redraw, set_event_sink};
pub use geometry::{Frames, Rect, frames};
pub use host::{
    AttachError, ViewId, attach, begin_frame, content_view, end_frame, hide, is_attached, is_mounted, is_suppressed,
    mount, place, scale, set_suppressed, unmount, view, views_in,
};
pub use keys::{focus, focus_host, focused_view, install_edit_menu, set_accepts_tab, set_action, set_menu_action};
pub use snapshot::{Snapshot, snapshot};
