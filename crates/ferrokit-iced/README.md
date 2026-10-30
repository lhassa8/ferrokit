# ferrokit-iced (alpha)

Ferrokit for Iced: real AppKit controls inside [Iced](https://iced.rs) 0.14 windows on macOS. (Building a whole Mac app without Iced? Use [`ferrokit`](https://crates.io/crates/ferrokit).) Not imitations: an actual `NSTableView` in your Iced layout, so you get native scrolling, keyboard navigation, type-select, Dark Mode and VoiceOver for free.

Status: **0.4.0** (formerly `iced-appkit`). Part of [Ferrokit](https://github.com/lhassa8/ferrokit); its crates share one version number. macOS 15 or later, Iced 0.14. Two widgets: a table (`NSTableView`) and a plain-text editor (`NSTextView`).

## Why

- **Large data:** Iced's own `table` builds every row. At 100,000 rows it uses 1.3 GB and scrolls at about 20 fps. `ferrokit_iced::table` shows 1,000,000 rows at 60 fps with flat memory, because AppKit only asks for the rows on screen.
- **Accessibility:** a native table exposes rows, columns, headers and cell text to VoiceOver.

## Setup (once per app)

```rust
// 1. Wrap the window's root element (frame bookkeeping, and Iced dropdowns,
//    tooltips and modals drawn above native views).
fn view(&self) -> Element<'_, Message> {
    ferrokit_iced::root(content).into()
}

// 2. Attach when the window opens, and route native events.
fn subscription(&self) -> Subscription<Message> {
    Subscription::batch([
        iced::window::open_events().map(Message::WindowOpened),
        ferrokit_iced::events().map(Message::Native),
    ])
}

fn update(&mut self, message: Message) -> Task<Message> {
    match message {
        Message::WindowOpened(id) => ferrokit_iced::attach(id, |_| Message::Attached),
        Message::Native(event) => ferrokit_iced::update(&event),
        // ...
    }
}
```

Optional:
- `ferrokit_iced::theme::system()` makes Iced use the system colours and accent colour, and follow Dark Mode.
- `ferrokit_iced::host::install_edit_menu()` adds a standard Edit menu that routes ⌘C/⌘V/⌘Z to whichever side has focus.

## A table

```rust
use ferrokit_iced::table::{Column, TableModel};
use ferrokit_iced::ViewId;

let people = TableModel::new(ViewId(1), vec![
    Column::text("name", "Name", |p: &Person| p.name.clone()).width(220.0),
    Column::number("age", "Age", |p: &Person| p.age as f64),
    Column::check("active", "Active", |p: &Person| p.active),
]);
people.set_rows(rows);

// in view():
ferrokit_iced::table(&self.people)
    .on_select(Message::Selected)   // model indices
    .on_activate(Message::Open)     // double-click or Return
    .on_sort(Message::Sorted)       // already re-sorted (natural order)
    .on_toggle(Message::Toggled)    // (row, column id, value)
```

The model owns the native view, so keep it in your state. Create it once and update it in place with `set_rows`, `update_row`, `insert` and `remove`.

## A text editor

```rust
use ferrokit_iced::text::TextModel;

let notes = TextModel::new(ViewId(2), "Initial text");

// in view():
ferrokit_iced::text_editor(&self.notes)
    .on_change(Message::NotesChanged)   // the whole text; not sent mid-IME
    .on_composition(Message::Composing) // Some(marked text) while an IME composes
```

You get what a Mac text view does: Japanese/Chinese/Korean input with the candidate window at the caret, spell and grammar checking with suggestions, Look Up and Translate, the find bar (⌘F), undo, Dark Mode and VoiceOver. Each editor keeps its own undo history. Tab inserts a tab; Ctrl-Tab moves focus on (`set_accepts_tab(false)` makes plain Tab move on). Offsets in the API are UTF-8 byte ranges.

## Things to know

- **Iced overlays and modals:**
  - While an Iced overlay covers a native view, `root` swaps the view for a picture of itself and hides it, so the overlay draws on top. Overlays appear about 2 frames after opening.
  - Wrap a modal layer in `ferrokit_iced::cover(...)` so it gets the same treatment.
- **Focus:**
  - Native views take part in Tab order (`NativeSlot::focusable`, on by default for tables).
  - Iced's `focus_next` stops at the last widget; `ferrokit_iced::update` wraps around.
- **Themes:** any Iced theme pins the window's light/dark appearance to the theme. Use `theme::system()` if native views should follow the system.

## Examples

- `cargo run --release -p ferrokit-iced --example csv_browser [file.csv]`: a CSV browser with filtering, sorting, a details modal, a native Open sheet and a native notes editor.
- `cargo run --release -p ferrokit-iced --example checklist`: the regression suite. Run with `CHECK=all` for the unattended checks.

License: MIT OR Apache-2.0.
