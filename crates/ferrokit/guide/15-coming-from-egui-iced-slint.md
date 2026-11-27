# Coming from egui, Iced or Slint

If you've built a GUI in Rust before, most of Ferrokit will feel familiar.
Two things are different, and the rest follows from them:

- **The views are AppKit's.** A `button` is an `NSButton` and a `table` is an
  `NSTableView`. Ferrokit doesn't draw widgets, and nothing renders a frame
  on the GPU. That's why it's Mac only, and why VoiceOver, text input,
  Dark Mode and the system's tables work without extra code.
- **Views are built once, then updated in place.** egui runs your UI code
  every frame and Iced calls `view` after every message. Ferrokit runs
  your view code once, when the window opens. The parts that change are
  closures over *signals*, and when a signal changes, only the views that
  read it update.

## The same ideas, side by side

| | egui | Iced | Slint | Ferrokit |
| --- | --- | --- | --- | --- |
| State | fields of your app struct, read each frame | the state struct `update` changes | `property`s in the `.slint` file | `Signal<T>`: a `Copy` handle, read with `get()`, changed with `set`/`update` |
| Events | `if ui.button(..).clicked()` | a `Message` enum, matched in `update` | callbacks (`clicked => { .. }`) | closures: `button("Add", move \|\| ..)` |
| Describing the UI | code that runs every frame | `view(&self) -> Element`, rebuilt after each message | `.slint` markup | functions returning `impl View`, run once |
| What changes on screen | everything, every frame | the diff of the new `view` | bound properties | each view reading a changed signal, and nothing else |
| Layout | `ui.horizontal`, `ui.vertical` | `row!`, `column!` | `HorizontalLayout`, `VerticalLayout` | `hstack`, `vstack`, `grid`, `spacer`, `.padding(..)`, `.frame(..)` |
| Lists | a `for` loop | `column(items.iter().map(..))` | `for item in items` | `for_each(items, row)`, keyed by `Identifiable` |
| Tables | `egui_extras::TableBuilder` | `table` (0.14) | `StandardTableView` | `table(&model)`: an `NSTableView`, a million rows |
| Background work | your own threads | `Task::perform` | `spawn_local`, threads | `spawn`, `background(..).await` in `spawn_local`, `Resource::new` |
| Windows | `eframe::run_native` | `iced::application` | one component per window | `App::new(..).window(Window::new(..))`, `open(..)` |
| Look | `Visuals` | themes | styles and properties | the system's, plus `.font(..)`, `.color(..)`, `.background(..)`, `.corner_radius(..)` |
| Tests | `egui_kittest` | `iced_test` | the testing backend | self-checks that drive the real controls (`ferrokit::testing`) |

## One app, four ways

A counter: a label and a button. In **egui**, the UI is code that runs every
frame, and the button's result is a `bool` you test:

```rust,ignore
struct Counter { count: i32 }

impl eframe::App for Counter {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label(format!("Clicked {} times", self.count));
            if ui.button("Click me").clicked() {
                self.count += 1;
            }
        });
    }
}
```

In **Iced**, a click sends a message, `update` changes the state, and `view`
builds the whole tree again:

```rust,ignore
#[derive(Default)]
struct Counter { count: i32 }

#[derive(Debug, Clone)]
enum Message { Clicked }

impl Counter {
    fn update(&mut self, message: Message) {
        match message {
            Message::Clicked => self.count += 1,
        }
    }

    fn view(&self) -> Element<'_, Message> {
        column![
            text(format!("Clicked {} times", self.count)),
            button("Click me").on_press(Message::Clicked),
        ]
        .padding(20)
        .into()
    }
}
```

In **Slint**, the UI is markup with properties, and Rust code drives it from
outside:

```slint
export component Counter inherits Window {
    in-out property <int> count: 0;
    VerticalLayout {
        Text { text: "Clicked \{count} times"; }
        Button { text: "Click me"; clicked => { count += 1; } }
    }
}
```

In **Ferrokit**, the state is a signal. The label is given a *closure*, so
it follows the signal, and the button is given the action to run:

```rust,no_run
use ferrokit::*;

fn main() {
    let count = Signal::new(0);
    App::new("Counter")
        .window(Window::new("Counter").size(320.0, 140.0).view(
            vstack((
                text(move || format!("Clicked {} times", count.get())),
                button("Click me", move || count.update(|n| *n += 1)),
            ))
            .padding(20.0),
        ))
        .run();
}
```

There's no `Message` enum and no `update` function. A click runs its
closure, the closure changes `count`, and the one `NSTextField` that reads
`count` gets new text. The stack, the button and the window are never
touched again.

## A bigger example: a to-do list

Here's a list you can add to and clear items from, with a count. Where Iced
would have `Message::DraftChanged`, `Message::Add` and `Message::Remove(id)`,
here each control gets a signal or a closure:

```rust,no_run
use ferrokit::*;

#[derive(Clone)]
struct Todo {
    id: u32,
    title: String,
}

impl Identifiable for Todo {
    type Id = u32;
    fn id(&self) -> u32 {
        self.id
    }
}

fn main() {
    let todos = Signal::new(Vec::<Todo>::new());
    let draft = Signal::new(String::new());
    let next_id = Signal::new(1);
    // A closure that captures only signals is `Copy`: use it twice.
    let add = move || {
        let title = draft.get_untracked().trim().to_string();
        if title.is_empty() {
            return;
        }
        let id = next_id.get_untracked();
        next_id.set(id + 1);
        todos.update(|v| v.push(Todo { id, title }));
        draft.set(String::new());
    };
    let list = for_each(move || todos.get(), move |todo: Todo| {
        let id = todo.id;
        hstack((
            text(todo.title),
            spacer(),
            button("Done", move || todos.update(|v| v.retain(|t| t.id != id))),
        ))
    });
    App::new("Todos")
        .window(Window::new("Todos").size(360.0, 400.0).view(
            vstack((
                hstack((
                    text_field(&draft).placeholder("New to-do").on_submit(add),
                    button("Add", add).disabled(move || draft.with(|d| d.trim().is_empty())),
                )),
                scroll(vstack((list,))),
                text(move || format!("{} left", todos.with(Vec::len))),
            ))
            .padding(16.0),
        ))
        .run();
}
```

What to notice, coming from the other toolkits:

- **`text_field(&draft)` is bound both ways.** Typing changes `draft`, and
  `draft.set(..)` changes the field. There's no `on_input` message to
  handle.
- **`for_each` keeps rows by identity.** Adding a to-do builds one row, and
  removing one drops one. The others, their `NSView`s included, stay as
  they are, so a row's focus or scroll position isn't lost.
- **`get()` tracks and `get_untracked()` doesn't.** Inside a view's closure
  (`text(move || ..)`, `.disabled(move || ..)`), `get()` subscribes the view
  to the signal. In an action you just want the value, so use
  `get_untracked()`.
- **`button("Add", add).disabled(..)` takes a closure**, so the button
  enables and disables itself as `draft` changes.

## Habits to unlearn

- **A value isn't a binding.** `text(format!("{}", count.get()))` reads the
  count once, when the window opens, and never changes. Give `text` a
  closure: `text(move || format!("{}", count.get()))`. The same goes for
  every modifier that takes `impl IntoValue`: a plain value is fixed, and
  a closure follows its signals.
- **Don't rebuild views to change them.** In Iced the whole tree is rebuilt
  after each message; here, building a new view means a new AppKit view.
  That's slow, and it loses focus and selection. Put the changing part in a
  closure, or in `dynamic(..)` when a whole subtree has to change.
- **No per-frame loop.** Nothing runs unless a signal changes or an event
  arrives. For a clock or a live feed, `on_frame` calls you once per
  display frame, and a background thread can `spawn` its results back.
- **Main thread only.** AppKit is single-threaded, and so are signals and
  views. Heavy work goes to `spawn` or `background(..)`, which hand the
  result back to the main thread.
- **Most styling is already done.** Controls look and behave as macOS
  draws them, in light and Dark Mode, at every text size. Reach for
  `.font`, `.color`, `.background` and `.corner_radius` when you want your
  own look, not to make a button look like a button.

## Where to go next

- [State](https://docs.rs/ferrokit/latest/ferrokit/guide/ch02_state/): signals, memos, effects, and
  async work, in depth.
- [Views and layout](https://docs.rs/ferrokit/latest/ferrokit/guide/ch03_views_and_layout/): stacks, grids, lists
  and identity.
- [Tables, outlines and editors](https://docs.rs/ferrokit/latest/ferrokit/guide/ch05_tables_outlines_editors/):
  what replaces `TableBuilder` and `StandardTableView`.
- [Testing and shipping](https://docs.rs/ferrokit/latest/ferrokit/guide/ch08_testing_and_shipping/): self-checks
  and getting the app to people.
