# More controls and charts

This chapter covers the rest of AppKit's standard controls (tabs, radio
buttons, links, gauges and ratings, token fields, path controls and
tooltips) and the full set of charts. The `components` example is a trip
budget that uses all of them:

```bash
cargo run -p ferrokit --example components
```

## Tabs

`tabs` shows one of several panes, bound two ways to a signal. Each page
is a title, the value that selects it, and its content:

```rust,no_run
use ferrokit::*;

let page = Signal::new(0);
let view = tabs(&page, vec![
    ("Overview", 0, AnyView::new(text("Charts here"))),
    ("Settings", 1, AnyView::new(text("Controls here"))),
])
.style(TabStyle::Segmented);
# let _ = view;
```

- **`TabStyle::Segmented`:** a segmented control above borderless content,
  as most modern Mac apps do.
- **`TabStyle::Bordered`:** the default. AppKit's tab view, with tabs along
  the top of a bordered box, as in inspectors and older settings panes.

Every pane is built once and keeps its state while hidden. A pane sits at
the top left of the tab view, unless it fills it (a chart, a list).

## Choices and values

```rust,no_run
use ferrokit::*;

let currency = Signal::new("USD".to_string());
let tags = Signal::new(vec!["Hiking".to_string()]);
let stars = Signal::new(4.0);
let used = Signal::new(62.0);
let folder = Signal::new(std::env::temp_dir());

let form = vstack((
    radio_group(&currency, vec![("US dollar", "USD".to_string()), ("Euro", "EUR".to_string())]).horizontal(),
    token_field(&tags).suggestions(&["Hiking", "Hotels", "Islands"]).placeholder("Add tags"),
    rating(&stars, 5).accessibility_label("Rating"),
    gauge(move || used.get(), 100.0).warning(75.0).critical(90.0).accessibility_label("Storage"),
    path_control(&folder).accessibility_label("Export folder"),
    link("Read the guide", "https://github.com/lhassa8/ferrokit"),
));
# let _ = form;
```

- **`radio_group`:** radio buttons choosing one value, one above another or
  `.horizontal()`.
- **`token_field`:** words or names as tokens (tags, recipients), finished
  with Return or a comma. `.suggestions(..)` offers completions as the user
  types.
- **`rating`:** stars the user clicks, as in Music and Photos.
- **`gauge`:** a level from 0 to a maximum (storage, a battery).
  `.warning(..)` and `.critical(..)` turn it yellow and red, and
  `.discrete()` draws segments.
- **`path_control`:** the folders down to a file, as the Finder's path bar
  shows them. Clicking a folder chooses it, and when there isn't room the
  middle folders shorten to fit.
- **`link`:** text that opens a web page, a `mailto:` address or a file in
  its app. VoiceOver reads it as a link and can open it.

## Tooltips

`.tooltip(..)` works on any view: the text appears when the pointer rests
on it, and VoiceOver reads it as the view's help. SwiftUI calls this
`help`.

```rust,no_run
use ferrokit::*;

let reset = button("Reset", || {}).tooltip("Clears the rating");
# let _ = reset;
```

## Charts

Every chart is drawn natively, redraws when the signals its data function
reads change, shows values under the pointer, and has a legend when it has
more than one series. VoiceOver reads each one as an image, described by
a summary.

| Chart | Data | For |
| --- | --- | --- |
| `bar_chart` | `(label, value)` pairs | comparing categories |
| `stacked_bar_chart` | `(label, values)`, plus `.series_names(..)` | parts of each category (`.grouped()` puts them side by side) |
| `line_chart` | `Series` of `(x, y)` points | change over time; millions of points draw quickly |
| `area_chart` | `Series` | totals over time, filled down to zero |
| `scatter_chart` | `Series` | how two values relate |
| `pie_chart` | `(label, value)` pairs | shares of a whole (`.donut()` for a ring with the total inside) |

```rust,no_run
use ferrokit::*;

const DAY: f64 = 86_400.0;
let balance: Vec<(f64, f64)> = (0..90).map(|d| (1_767_225_600.0 + d as f64 * DAY, 4200.0 - d as f64 * 31.0)).collect();
let chart = area_chart(move || vec![Series::new("Balance", balance.clone()).color(Color::Green)])
    .value_format(Format::Currency("USD".into()))
    .x_format(Format::Date);

let spending = pie_chart(|| vec![("Lodging".to_string(), 1240.0), ("Food".to_string(), 520.0)])
    .donut()
    .value_format(Format::Currency("USD".into()))
    .colors(&[Color::Indigo, Color::Orange]);
# let _ = (chart, spending);
```

**Formats**, for values (`.value_format(..)`) and for the x axis of line,
area and scatter charts (`.x_format(..)`):
- **`Format::Number`:** the default; large numbers are compact on the axis
  (12k, 1.5M).
- **`Format::Decimals(n)`:** fixed decimals, grouped as the user's locale
  does.
- **`Format::Currency("USD")`:** money as the user's locale writes it.
- **`Format::Percent`:** fractions as percentages (0.25 is 25%).
- **`Format::Date` and `Format::DateTime`:** values are seconds since 1970.
  Ticks fall on whole hours, days, Mondays, months or years in the user's
  time zone, through daylight saving changes.

**Colours:** `.colors(&[..])` sets the series' (or bars' or slices')
colours in order, and `Series::color(..)` gives one series its own.
`.no_legend()` hides the legend.
