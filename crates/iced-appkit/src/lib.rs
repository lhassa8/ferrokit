//! Real AppKit views inside Iced 0.14 windows on macOS.
//!
//! Wiring, once per app:
//! - wrap the window's root element in [`root`] (frame bookkeeping, and native
//!   views swapped for pictures under Iced overlays);
//! - run [`attach`] once the window exists;
//! - subscribe to [`events`] and pass each event to [`update`] (focus order,
//!   redraws); the event is also yours to act on (e.g. `Event::Action`).

#![warn(missing_docs)]

mod root;
mod slot;
pub mod table;
pub mod text;
pub mod theme;

use std::sync::Mutex;

use iced::advanced::widget::operation::Focusable;
use iced::advanced::widget::{Id, Operation};
use iced::futures::channel::mpsc;
use iced::{Rectangle, Subscription, Task};

pub use appkit_host::{self as host, Event, Rect, ViewId};
pub use root::{Cover, Root, cover, root};
pub use slot::NativeSlot;
pub use table::{Table, table};
pub use text::{TextEditor, text_editor};

static SENDER: Mutex<Option<mpsc::UnboundedSender<Event>>> = Mutex::new(None);

/// Attaches to an Iced window. Run the returned task once the window exists
/// (e.g. on `window::open_events()`); `then` receives whether it worked.
pub fn attach<T: Send + 'static>(window: iced::window::Id, then: impl FnOnce(bool) -> T + Send + 'static) -> Task<T> {
    iced::window::run(window, move |w| {
        let ok = appkit_host::attach(&w).is_ok();
        if ok {
            appkit_host::set_event_sink(|event| {
                if let Some(tx) = SENDER.lock().unwrap().as_ref() {
                    let _ = tx.unbounded_send(event);
                }
            });
        }
        then(ok)
    })
}

/// Native events (focus steps, focus changes, control actions, appearance
/// changes, redraw requests). Pass each to [`update`].
pub fn events() -> Subscription<Event> {
    Subscription::run(|| {
        let (tx, rx) = mpsc::unbounded();
        *SENDER.lock().unwrap() = Some(tx);
        rx
    })
}

/// The library's share of handling a native event: moves Iced focus.
pub fn update<T: Send + 'static>(event: &Event) -> Task<T> {
    match event {
        Event::FocusStep { forward } => focus_step(*forward),
        // Focus went into a native view (e.g. a click): Iced widgets must let
        // go, or two widgets would look focused.
        Event::FocusChanged(Some(_)) => iced::advanced::widget::operate(UnfocusIced),
        _ => Task::none(),
    }
}

thread_local! {
    /// Widget ids handed out to native slots.
    static SLOT_IDS: std::cell::RefCell<Vec<Id>> = const { std::cell::RefCell::new(Vec::new()) };
}

pub(crate) fn slot_widget_id(id: ViewId) -> Id {
    let wid = Id::from(format!("iced-appkit/{}", id.0));
    SLOT_IDS.with_borrow_mut(|ids| {
        if !ids.contains(&wid) {
            ids.push(wid.clone());
        }
    });
    wid
}

fn is_slot(id: Option<&Id>) -> bool {
    id.is_some_and(|id| SLOT_IDS.with_borrow(|ids| ids.contains(id)))
}

/// Moves focus to the next (or previous) focusable widget, native slots
/// included, wrapping around at the ends. (Iced's own `focus_next` stops at
/// the end, leaving nothing focused.)
pub fn focus_step<T: Send + 'static>(forward: bool) -> Task<T> {
    use iced::advanced::widget::operation::{self, focusable::Count};

    struct Step {
        count: Count,
        current: usize,
        forward: bool,
    }

    impl<T> Operation<T> for Step {
        fn focusable(&mut self, _id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
            let total = self.count.total;
            if total == 0 {
                return;
            }
            let target = match (self.count.focused, self.forward) {
                (None, true) => 0,
                (None, false) => total - 1,
                (Some(f), true) => (f + 1) % total,
                (Some(f), false) => (f + total - 1) % total,
            };
            if self.count.focused == Some(self.current) && self.current != target {
                state.unfocus();
            }
            if self.current == target {
                state.focus();
            }
            self.current += 1;
        }

        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<T>)) {
            operate(self);
        }
    }

    // `then` takes a fn pointer, so one per direction.
    fn next(count: Count) -> Step {
        Step { count, current: 0, forward: true }
    }
    fn previous(count: Count) -> Step {
        Step { count, current: 0, forward: false }
    }
    let then = if forward { next } else { previous };
    iced::advanced::widget::operate(operation::then(operation::focusable::count(), then))
}

/// Unfocuses every focusable Iced widget except native slots.
struct UnfocusIced;

impl<T> Operation<T> for UnfocusIced {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if !is_slot(id) && state.is_focused() {
            state.unfocus();
        }
    }

    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<T>)) {
        operate(self);
    }
}
