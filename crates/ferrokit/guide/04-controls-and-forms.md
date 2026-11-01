# Controls and forms

Each control is the AppKit one, bound two ways to a signal: the control
shows the signal's value, and user changes set the signal.

```rust,no_run
use ferrokit::*;

let name = Signal::new(String::new());
let password = Signal::new(String::new());
let notify = Signal::new(true);
let sounds = Signal::new(false);
let size = Signal::new(2u8);
let volume = Signal::new(0.5);
let days = Signal::new(30.0);

let form = vstack((
    text_field(&name).placeholder("Your name").accessibility_label("Name"),
    secure_field(&password).accessibility_label("Password"),
    toggle("Notifications", &notify),          // an NSSwitch with a title
    checkbox("Play sounds", &sounds),
    picker(&size, vec![("Small", 1u8), ("Medium", 2), ("Large", 3)]).accessibility_label("Size"),
    slider(&volume, 0.0, 1.0).accessibility_label("Volume"),
    stepper(&days, 1.0, 365.0, 1.0).accessibility_label("Keep for"),
    progress(|| Some(0.4)),                     // None: indeterminate
    image("star.fill").description("Favourite"),
));
# let _ = form;
```

| Control | AppKit view | Value |
| --- | --- | --- |
| `text_field`, `secure_field` | `NSTextField`, `NSSecureTextField` | `Signal<String>` |
| `toggle` | `NSSwitch` | `Signal<bool>` |
| `checkbox` | `NSButton` (check box) | `Signal<bool>` |
| `picker` | `NSPopUpButton` | any `Signal<T: PartialEq + Clone>` |
| `slider`, `stepper` | `NSSlider`, `NSStepper` | `Signal<f64>` |
| `progress` | `NSProgressIndicator` | a closure: `Some(fraction)` or `None` |
| `image` | `NSImageView` with an SF Symbol | – |
| `button` | `NSButton` | an action |
| `menu_button` | `NSPopUpButton`, pulling down | menu items |
| `help_button` | `NSButton`, help bezel | an action |
| `date_picker` | `NSDatePicker` | `Signal<SystemTime>` |
| `time_picker` | `NSDatePicker` (hour and minute) | `Signal<(u32, u32)>` |
| `color_well` | `NSColorWell` | `Signal<Color>` |
| `segmented` | `NSSegmentedControl` | any `Signal<T: PartialEq + Clone>` |
| `combo_box` | `NSComboBox` | `Signal<String>`, with suggestions |
| `search_field` | `NSSearchField` | `Signal<String>` |

## Dates, colours, segments, suggestions, search

```rust,no_run
use std::time::SystemTime;
use ferrokit::*;

#[derive(Clone, PartialEq)]
enum Layout { List, Grid }

let due = Signal::new(SystemTime::now());
let remind_at = Signal::new((8u32, 30u32));
let tint = Signal::new(Color::rgb(0.2, 0.5, 1.0));
let layout = Signal::new(Layout::List);
let city = Signal::new(String::new());
let query = Signal::new(String::new());

let form = vstack((
    date_picker(&due).with_time().accessibility_label("Due"),       // .calendar(), .range(min, max)
    time_picker(&remind_at).accessibility_label("Remind at"),
    color_well(&tint).accessibility_label("Tint"),
    segmented(&layout, vec![("List", Layout::List), ("Grid", Layout::Grid)]).accessibility_label("Layout"),
    combo_box(&city, vec!["Berlin", "Lagos", "Tokyo"]).accessibility_label("City"),
    search_field(&query).placeholder("Search orders").on_submit(|| println!("search")),
));
# let _ = form;
```

- `date_picker` holds a `std::time::SystemTime`; `.calendar()` shows a
  month calendar instead of a field with a stepper.
- `time_picker` holds a time of day, `(hour, minute)` with the hour 0–23,
  shown in the user's 12- or 24-hour format: for a daily reminder or an
  opening time, which keep their hour and minute whatever the date.
- `Color` is sRGB (`Color::rgb(r, g, b)`, components 0–1); the well opens
  the system colour picker.
- `segmented` reads each segment's title to VoiceOver; give the control as
  a whole a name too.
- `combo_box` keeps free text: typing sets the signal, and so does picking
  a suggestion.
- `search_field` updates its signal as the user types; the clear button
  empties it, and Return runs `on_submit`.

## Buttons

```rust,no_run
use ferrokit::*;

let name = Signal::new(String::new());
let save = button("Save", || println!("saved"))
    .disabled(move || name.with(String::is_empty))  // re-evaluated when `name` changes
    .default_action();                           // Return presses it
# let _ = save;
```

`text_field(…).on_submit(f)` runs `f` on Return in that field.

A button's look and size, a menu of actions, and the help button:

```rust,no_run
use ferrokit::*;

let bar = hstack((
    button("Unread", || {}).style(ButtonStyle::Recessed),         // a filter bar's look
    button("Delete", || {}).destructive().control_size(ControlSize::Small),
    menu_button("Export", vec![
        menu_item("CSV…", || println!("csv")),
        menu_item("PDF…", || println!("pdf")),
    ]),
    menu_button("", vec![menu_item("Rename", || {})])
        .symbol("ellipsis.circle")
        .style(ButtonStyle::Borderless)
        .accessibility_label("More"),
    help_button(|| println!("help")),
));
# let _ = bar;
```

- `ButtonStyle`: `Standard`, `Borderless`, `Inline` (a capsule inside a
  row), `Recessed`, `Circular` (a symbol alone), `Toolbar`, `Glass` (Liquid
  Glass on macOS 26; standard before).
- `ControlSize`: `Mini`, `Small` (inspectors), `Regular`, `Large`.
- `.destructive()` marks Delete and Discard: AppKit never presses it for
  Return, and draws it red where the style does.
- `menu_button` items are the menu bar's: `.disabled(..)`, `.checked(..)`,
  key equivalents and separators work the same.

## Sliders and progress

```rust,no_run
use ferrokit::*;

let quality = Signal::new(50.0);
let volume = Signal::new(0.5);
let angle = Signal::new(0.0);
let loaded = Signal::new(None::<f64>);

let panel = hstack((
    slider(&quality, 0.0, 100.0).ticks(11).snap_to_ticks(),   // stops at 0, 10, … 100
    slider(&volume, 0.0, 1.0).vertical(),                     // fills the height it's given
    slider(&angle, 0.0, 360.0).circular(),                    // a dial
    progress(move || loaded.get()).spinner().control_size(ControlSize::Small),
));
# let _ = panel;
```

A spinner spins while its closure returns `None` and fills a circle with
`Some(fraction)`; a bar does the same as a stripe.

## Tab order, shortcuts, input methods

Tab and Shift-Tab follow the views' positions, top-left to bottom-right.
Text fields get AppKit's own editing: ⌘A, ⌘C, ⌘V, ⌘Z, spell checking,
dictation, and input methods (Japanese, Chinese…) with the candidate window
at the caret. You don't write any of it.

## Accessibility: name every control

VoiceOver reads a control's **name**. A field after `text("Name:")` has none
of its own: the text is a separate view. Name such controls with
`.accessibility_label("…")`:

- a `text_field`, `picker`, `slider` or `stepper` beside a text label
- an editor or a table that isn't otherwise titled
- an icon-only button

`button("Save", …)`, `checkbox("…", …)` and `toggle("…", …)` are named by
their titles. `image(symbol)` is decorative (VoiceOver skips it) unless you
give it a `.description("…")`.

`testing::ax_audit(&window)` lists the controls VoiceOver can't name; run
it in your app's self-checks (see [Testing and shipping](08-testing-and-shipping.md)).
