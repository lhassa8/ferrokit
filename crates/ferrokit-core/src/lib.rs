//! The AppKit layer under [`ferrokit`](https://docs.rs/ferrokit): apps want
//! that crate.
//!
//! Places real AppKit views in a window and keeps their frames, focus and
//! keyboard in step with `ferrokit`'s layout. [`table::TableModel`] and
//! [`text::TextModel`], which `ferrokit` re-exports, are documented here.
//!
//! Main thread only. Every function panics off the main thread.

#![warn(missing_docs)]
#![doc(html_logo_url = "https://raw.githubusercontent.com/lhassa8/ferrokit/main/assets/icon.png", html_favicon_url = "https://raw.githubusercontent.com/lhassa8/ferrokit/main/assets/icon-128.png")]

mod appearance;
mod events;
mod geometry;
mod host;
#[doc(hidden)]
pub mod kept;
mod keys;
mod snapshot;
pub mod sync;
pub mod table;
pub mod text;
pub mod testing;

pub use appearance::{Rgba, SystemColors, generation as appearance_generation, is_dark, observe_appearance, system_colors};
pub use events::{Event, request_redraw, set_event_sink};
pub use geometry::{Frames, Rect, frames};
pub use host::{
    AttachError, ViewId, attach, begin_frame, content_view, end_frame, hide, is_attached, is_mounted, is_suppressed,
    mount, place, scale, set_suppressed, unmount, view, views_in,
};
pub use keys::{focus, focus_host, focused_view, install_edit_menu, set_accepts_tab, set_action, set_menu_action};
pub use snapshot::{Snapshot, snapshot};
