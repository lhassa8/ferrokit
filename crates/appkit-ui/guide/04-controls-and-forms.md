# Controls and forms

Each control is the AppKit one, bound two ways to a signal: the control
shows the signal's value, and user changes set the signal.

```rust,no_run
use appkit_ui::*;

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
| `date_picker` | `NSDatePicker` | `Signal<SystemTime>` |
| `color_well` | `NSColorWell` | `Signal<Color>` |
| `segmented` | `NSSegmentedControl` | any `Signal<T: PartialEq + Clone>` |
| `combo_box` | `NSComboBox` | `Signal<String>`, with suggestions |
| `search_field` | `NSSearchField` | `Signal<String>` |

## Dates, colours, segments, suggestions, search

```rust,no_run
use std::time::SystemTime;
use appkit_ui::*;

#[derive(Clone, PartialEq)]
enum Layout { List, Grid }

let due = Signal::new(SystemTime::now());
let tint = Signal::new(Color::rgb(0.2, 0.5, 1.0));
let layout = Signal::new(Layout::List);
let city = Signal::new(String::new());
let query = Signal::new(String::new());

let form = vstack((
    date_picker(&due).with_time().accessibility_label("Due"),       // .calendar(), .range(min, max)
    color_well(&tint).accessibility_label("Tint"),
    segmented(&layout, vec![("List", Layout::List), ("Grid", Layout::Grid)]).accessibility_label("Layout"),
    combo_box(&city, vec!["Berlin", "Lagos", "Tokyo"]).accessibility_label("City"),
    search_field(&query).placeholder("Search orders").on_submit(|| println!("search")),
));
# let _ = form;
```

- `date_picker` holds a `std::time::SystemTime`; `.calendar()` shows a
  month calendar instead of a field with a stepper.
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
use appkit_ui::*;

let name = Signal::new(String::new());
let save = button("Save", || println!("saved"))
    .disabled(move || name.with(String::is_empty))  // re-evaluated when `name` changes
    .default_action();                           // Return presses it
# let _ = save;
```

`text_field(…).on_submit(f)` runs `f` on Return in that field.

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
