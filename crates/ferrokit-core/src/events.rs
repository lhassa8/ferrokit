//! Callbacks from AppKit, handed to the host framework through one sink.

use std::cell::RefCell;

use crate::ViewId;

#[derive(Debug, Clone, PartialEq)]
/// A callback from AppKit for the host framework (see [`set_event_sink`]).
pub enum Event {
    /// Tab (`forward`) or Shift-Tab was pressed and belongs to the host's
    /// focus order (which spans host widgets and native views).
    FocusStep {
        /// Tab (true) or Shift-Tab (false).
        forward: bool,
    },
    /// Keyboard focus moved into a mounted native view (`Some`) or out of all
    /// of them (`None`), e.g. after a click.
    FocusChanged(Option<ViewId>),
    /// The system appearance or accent colour changed.
    AppearanceChanged,
    /// A native control registered with [`crate::set_action`] fired.
    Action(ViewId),
    /// The host should draw another frame (see [`crate::request_redraw`]).
    Redraw,
}

type Sink = Box<dyn Fn(Event)>;

thread_local! {
    static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
}

/// Where events go. Called on the main thread, from AppKit callbacks.
pub fn set_event_sink(sink: impl Fn(Event) + 'static) {
    SINK.with_borrow_mut(|s| *s = Some(Box::new(sink)));
}

pub(crate) fn emit(event: Event) {
    SINK.with_borrow(|s| {
        if let Some(sink) = s {
            sink(event);
        }
    });
}

thread_local! {
    static REDRAW_QUEUED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Asks the host for another frame: emits [`Event::Redraw`] once per frame
/// (the flag clears at [`crate::begin_frame`]). Used while a snapshot swap is
/// in progress and no input event would otherwise cause a redraw.
pub fn request_redraw() {
    if !REDRAW_QUEUED.with(|q| q.replace(true)) {
        emit(Event::Redraw);
    }
}

pub(crate) fn frame_started() {
    REDRAW_QUEUED.with(|q| q.set(false));
}
