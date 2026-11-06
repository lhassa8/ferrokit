//! Callbacks from AppKit, handed to the host framework through one sink.

use crate::kept::{UiCell, UiLocal};

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
    static SINK: UiCell<Option<Sink>> = const { UiCell::new(None) };
}

/// Where events go. Called on the main thread, from AppKit callbacks.
pub fn set_event_sink(sink: impl Fn(Event) + 'static) {
    SINK.with_borrow_mut(|s| *s = Some(Box::new(sink)));
}

/// Sends `event` to the sink; false if there's none yet.
pub(crate) fn emit(event: Event) -> bool {
    SINK.with_borrow(|s| match s {
        Some(sink) => {
            sink(event);
            true
        }
        None => false,
    })
}

thread_local! {
    static REDRAW_QUEUED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Asks the host for another frame: emits [`Event::Redraw`] once per frame
/// (the flag clears at [`crate::begin_frame`]). Used while a snapshot swap is
/// in progress and no input event would otherwise cause a redraw.
pub fn request_redraw() {
    // A request before there's a sink (a model filled before the app runs)
    // reaches nobody: it mustn't leave the flag set, or no later request
    // would be sent and queued events would wait forever.
    if !REDRAW_QUEUED.with(|q| q.replace(true)) && !emit(Event::Redraw) {
        REDRAW_QUEUED.with(|q| q.set(false));
    }
}

pub(crate) fn frame_started() {
    REDRAW_QUEUED.with(|q| q.set(false));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    /// A redraw asked for before the sink exists (a table filled before the
    /// app runs) doesn't block the ones after it.
    #[test]
    fn an_early_redraw_request_doesnt_block_later_ones() {
        request_redraw();
        let redraws = Rc::new(std::cell::Cell::new(0));
        let r = redraws.clone();
        set_event_sink(move |e| {
            if e == Event::Redraw {
                r.set(r.get() + 1);
            }
        });
        request_redraw();
        request_redraw();
        assert_eq!(redraws.get(), 1, "one per frame");
        frame_started();
        request_redraw();
        assert_eq!(redraws.get(), 2);
    }
}
