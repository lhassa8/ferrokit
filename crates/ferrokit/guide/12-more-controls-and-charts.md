# More controls and charts

This chapter covers the rest of AppKit's standard controls (tabs, radio
buttons, links, gauges and ratings, token fields, path controls and
tooltips) and the full set of charts. The `components` example is a trip
budget that uses all of them:

```bash
git clone https://github.com/lhassa8/ferrokit && cd ferrokit
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

## Disabled controls

`.disabled(|| ..)` works on any view: its controls are disabled while the
function returns true, following the signals it reads. SwiftUI's
`disabled`.

```rust,no_run
use ferrokit::*;

let remind = Signal::new(false);
let hour = Signal::new(8);
let row = hstack((
    checkbox("Remind me at", &remind),
    picker(&hour, vec![("8:00", 8), ("9:00", 9)]).disabled(move || !remind.get()),
));
# let _ = row;
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
| `histogram` | values | how many fall in each range (about 10 round-number bins, or `.bins(n)`) |
| `heatmap` | `(column, row, value)` triples | a grid shaded by value: activity by weekday and week, by hour |
| `sparkline` | values | a small trend beside a number or in a row: no axes, the last value marked |

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
`.no_legend()` hides the legend. A heatmap shades its first colour, with
a scale from the lowest value to the highest above it.

**Selection:** `.selection(&signal)` makes a bar, stacked bar, pie,
histogram or heatmap chart selectable: clicking a mark sets the signal to
its index (a heatmap's cell is row × columns + column), clicking off the
marks to `None`, and the other marks dim while one is selected. On line,
area and scatter charts, `.x_selection(&signal)` sets the clicked x (the
nearest point's) and marks it with a line and each series' value. Both
work the other way too: setting the signal selects. VoiceOver's summary
says what's selected.

**Zooming and scrolling:** `.zoomable()` lets the user pinch (or ⌘-scroll)
a line, area or scatter chart to zoom in around the pointer, scroll
sideways to move along it, and pinch out to see it all again; vertical
scrolling still scrolls what the chart is in. `.x_range(&signal)` binds
the range shown (`None` is everything), so the app can zoom too. The value
axis fits what's shown, and a million points stay quick: only those in
range are drawn, a few per pixel column.

```rust,no_run
use ferrokit::*;

let picked = Signal::new(None::<usize>);
let months = bar_chart(|| vec![("Jan".to_string(), 1180.0), ("Feb".to_string(), 1110.0)]).selection(&picked);

let range = Signal::new(None::<(f64, f64)>);
let day = Signal::new(None::<f64>);
let readings: Vec<(f64, f64)> = (0..100_000).map(|i| (i as f64, (i as f64 / 500.0).sin())).collect();
let trace = line_chart(move || vec![Series::new("Signal", readings.clone())]).zoomable().x_range(&range).x_selection(&day);
# let _ = (months, trace);
```

**Rules** mark a threshold, a target or an average: `Rule::y(value)`
runs across the chart, `Rule::x(x)` down it (on line, area and scatter
charts and histograms). The chart's range takes them in, their labels sit
on a patch of the background so they read over the data, and VoiceOver's
summary includes them ("Budget at $1,300").

```rust,no_run
use ferrokit::*;

let minutes = vec![12.0, 18.0, 25.0, 31.0, 33.0, 47.0, 52.0];
let average = minutes.iter().sum::<f64>() / minutes.len() as f64;
let rides = histogram(move || minutes.clone()).bins(5).rule(Rule::x(average).label("Average"));

let sales = bar_chart(|| vec![("Mon".to_string(), 1200.0), ("Tue".to_string(), 800.0)])
    .rule(Rule::y(1000.0).label("Target").color(Color::Green));

let week = vec![3.0, 5.0, 4.0, 7.0, 6.0, 8.0, 9.0];
let trend = hstack((text("Orders"), sparkline(move || week.clone()).colors(&[Color::Green])));
# let _ = (rides, sales, trend);
```
