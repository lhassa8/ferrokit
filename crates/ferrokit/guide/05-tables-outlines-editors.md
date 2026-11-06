# Tables, outlines and editors

These are the widgets web views and custom-drawn toolkits find hardest, and
the reason to use AppKit: a virtualised `NSTableView` that scrolls millions
of rows at display rate, `NSOutlineView`, and `NSTextView` with all of the
system's text services.

## Tables

A table shows a `TableModel`: your rows (any type) and columns that read
from them. The model is created once and updated in place.

```rust,no_run
use std::rc::Rc;
use ferrokit::*;

#[derive(Clone)]
struct Order {
    customer: String,
    total: f64,
    paid: bool,
}

let model = Rc::new(TableModel::new(
    ViewId(1),
    vec![
        Column::text("customer", "Customer", |o: &Order| o.customer.clone()).width(200.0),
        Column::number("total", "Total", |o: &Order| o.total),
        Column::check("paid", "Paid", |o: &Order| o.paid),
    ],
));
model.set_rows(vec![Order { customer: "Ada".into(), total: 12.5, paid: true }]);

let m = model.clone();
let view = table(&model)
    .on_select(|rows| println!("selected model rows {rows:?}"))
    .on_activate(move |row| println!("opened {}", m.rows()[row].customer)) // double-click or Return
    .on_toggle(|row, column, on| println!("{column} of row {row} → {on}"))
    .accessibility_label("Orders");
# let _ = view;
```

- Row numbers are **model indices**: the position in what you passed to
  `set_rows`, whatever the current sort.
- Clicking a column header sorts (`Column::sortable(false)` opts out);
  `on_sort` tells you the new order.
- `.editable(true)` on a column makes its cells editable; `on_edit` reports
  `(row, column, new text)`.
- `update_row`, `append` and `select` change the model in place. Only the
  rows on screen have views; 200,000 rows load in about 150 ms.
- `Column::date(id, title, |row| Option<SystemTime>)` shows dates in the
  user's format and language ("Today", "12 Mar 2024"), sorted by date with
  rows without one first; `.with_time()` adds the time.
- `.on_key(TableKey::Delete, |rows| …)` and `TableKey::Space` take those
  keys without ⌘ while rows are selected (Delete to remove them, Space to
  Quick Look them, as in the Finder). Put the action in a menu too.
- `.selection(&rows)` binds the selection to a `Signal<Vec<usize>>` of
  model indices, both ways: clicking rows sets it, and setting it selects
  them. `set_rows` clears the selection and the signal with it, so set the
  signal after replacing the rows to select again.

### Groups, icons, progress bars, fitted columns

```rust,no_run
use std::rc::Rc;
use ferrokit::*;

#[derive(Clone)]
struct Download { name: String, day: String, done: f64 }

let model = Rc::new(TableModel::new(
    ViewId(3),
    vec![
        Column::text("name", "Name", |d: &Download| d.name.clone())
            .icon(|_| Some(CellImage::Symbol("doc".into())))   // or CellImage::FileIcon(path)
            .fit(),                                             // as wide as the longest name
        Column::progress("done", "Done", |d: &Download| d.done), // a bar from 0 to 1
    ],
));
model.set_rows(vec![Download { name: "report.pdf".into(), day: "Today".into(), done: 0.4 }]);
model.set_group_by(Some(Box::new(|d: &Download| d.day.clone())));  // a header per day
let view = table(&model).accessibility_label("Downloads");
# let _ = view;
```

- `set_group_by` shows a header row before each group. Headers are full width, float while their rows scroll, and can't be selected. Groups appear in the order of their first row; sorting by a column sorts within each group.
- Row indices in every event stay model indices, never header positions. `row_at(view_row)` is `None` for a header.
- A progress column sorts and copies as its number; VoiceOver reads the bar's value.
- `.fit()` measures the column's longest values (by character count, with many rows) whenever the rows change.

### Copy, paste, drag and drop

- ⌘C copies the selected rows as tab-separated text (Numbers and Excel
  paste it as cells).
- `.row_drag()` lets rows be dragged out.
- `.reorderable(|rows, before| …)` reorders them by dragging; apply the
  move with `model.move_rows(&rows, before)`.
- `.on_files_dropped(&["csv"], |paths, before| …)` takes files from the
  Finder.
- `.on_paste(|text| …)` handles ⌘V.

### Filtering: the filter builder

`filter_editor(&filter, fields)` is AppKit's filter builder (as in Mail's
rules and Finder's smart folders), editing a `Filter`: a line choosing
whether all, any or none of the rules must hold, then a row per rule, with
+ and − to add and remove them (Option with + adds a nested group). It
grows with its rows. Fields are `FilterField::text`, `number` or `choice`,
each with a key and a title.

A `Filter` is plain data. `filter.matches(|key| value)` tests a row, and
`filter.to_sql()` makes a SQL condition for a `WHERE`, with column names and
values quoted so what the user types can't change the query. A number rule
the user hasn't filled in yet is left out.

```rust,no_run
use ferrokit::*;

let filter = Signal::new(Filter::default());
let editor = filter_editor(&filter, vec![
    FilterField::text("name", "Name"),
    FilterField::number("total", "Total"),
    FilterField::choice("city", "City", &["Oslo", "Bergen", "Tromsø"]),
]);
let sql = move || format!("SELECT * FROM orders WHERE {};", filter.get().to_sql());
# let _ = (editor, sql);
```

The SQL browser's Filter… button opens one for the table shown.

## Outlines and the sidebar

`outline(roots, title)` shows a tree of `Identifiable` values; expansion and
selection survive updates. `sidebar(…)` is the same in the source-list
style of Mail's or Finder's sidebar: its roots are section headings, and
its rows follow the user's sidebar icon size (System Settings ›
Appearance). `.icon_color(|item| Some(Color::Green))` gives a row's symbol
its own colour, as Mail's mailboxes and Reminders' lists have.

```rust,no_run
use ferrokit::*;

#[derive(Clone)]
struct Folder {
    id: u32,
    name: String,
}

impl Identifiable for Folder {
    type Id = u32;
    fn id(&self) -> u32 {
        self.id
    }
}

let selected = Signal::new(None::<u32>);
let tree = || {
    vec![TreeNode::branch(
        Folder { id: 1, name: "Projects".into() },
        vec![TreeNode::leaf(Folder { id: 2, name: "ferrokit".into() })],
    )]
};
let view = sidebar(tree, |f: &Folder| f.name.clone()).icon(|_| Some("folder".into())).selection(&selected);
# let _ = view;
```

### Column browser

`column_browser(roots, title)` shows the same kind of tree as Finder's
column view (`NSBrowser`): the roots in the first column, and the selected
item's children in the next. `.selection(&path)` binds the selected item's
path, the ids from the first column down, both ways; setting it opens the
columns to that item. `.on_activate(|item| ..)` runs on a double-click, and
`.icon(|item| ..)` gives an SF Symbol per item. Arrow keys move through it
and typing selects an item by its title.

```rust,no_run
use ferrokit::*;

#[derive(Clone)]
struct Place(String);
impl Identifiable for Place {
    type Id = String;
    fn id(&self) -> String { self.0.clone() }
}
let p = |s: &str| Place(s.into());
let path = Signal::new(vec!["Norway".to_string(), "Lofoten".to_string()]);
let places = column_browser(move || vec![
    TreeNode::branch(p("Norway"), vec![TreeNode::branch(p("Lofoten"), vec![TreeNode::leaf(p("Reine"))])]),
], |place: &Place| place.0.clone())
.icon(|_| Some("mappin".into()))
.selection(&path)
.on_activate(|place| println!("open {}", place.0))
.accessibility_label("Places");
# let _ = places;
```

## Collections: grids of items

`collection(items, describe)` shows items in a grid (`NSCollectionView`),
as Finder's icon view or Photos do. Only the items on screen have views:
cells are made once and refilled as they scroll in, so 100,000 items load,
lay out and draw in about 5 ms.

```rust,no_run
use std::path::PathBuf;
use ferrokit::*;

#[derive(Clone)]
struct File(PathBuf);

impl Identifiable for File {
    type Id = PathBuf;
    fn id(&self) -> PathBuf {
        self.0.clone()
    }
}

let files = Signal::new(vec![File("/System/Applications/Calculator.app".into())]);
let selected = Signal::new(Vec::<PathBuf>::new());
let grid = collection(move || files.get(), |file: &File| {
    let name = file.0.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    CollectionItem::new(name, ItemImage::FileIcon(file.0.clone()))
})
.item_size(110.0, 104.0)
.selection(&selected)
.multiple()
.on_activate(|file| println!("open {:?}", file.0))   // double-click or Return
.accessibility_label("Files");
# let _ = grid;
```

- Each item shows a `CollectionItem`: a title, an optional subtitle and an
  image: `ItemImage::Symbol` (an SF Symbol), `FileIcon` (the Finder's icon
  for a path), `Color` (a swatch) or `None`.
- Items are keyed by `Identifiable` id: the selection (a signal of ids)
  follows its items when the list changes.
- Arrow keys, ⌘-click and ⇧-click work as in any Mac grid; VoiceOver reads
  each item by its title (and subtitle).

The `gallery` example browses a folder's apps with their icons, or 100,000
colour swatches.

## Text editor

`text_editor` shows a `TextModel` in an `NSTextView`: spell checking,
dictation, input methods, Find, undo, the Services menu.

```rust,no_run
use ferrokit::*;

let notes = TextModel::new(ViewId(2), "Meeting notes\n");
let text = Signal::new(String::new());
let editor = text_editor(&notes)
    .bind(&text)                                   // keep a signal in step
    .on_selection(|range| println!("selected bytes {range:?}"))
    .accessibility_label("Notes");
# let _ = editor;
```

Positions are UTF-8 byte offsets. Each editor has its own undo history;
`.shared_undo()` puts its typing in the window's history instead, between
your app's own undoable changes (see [The app shell](https://docs.rs/ferrokit/latest/ferrokit/guide/ch06_app_shell/)).

### Rich text

`.rich()` lets the user style the text, as in TextEdit: fonts and sizes
(the font panel, ⌘T), bold, italic and underline (⌘B, ⌘I, ⌘U), colours,
alignment, spacing and lists; pasted text keeps its styles, and pictures
can be pasted or dropped in. `.format_bar()` adds TextEdit's format bar
above the text and `.ruler()` the ruler; `format_menu()` is the Format menu
(Font and Text) to give the app.

`.bind_rtf(&signal)` binds the styled text two ways as RTF, what a rich
document saves. On the model, `rtf()` and `html()` read the text out with
its styles, and `set_rtf(..)` and `set_html(..)` load it.

```rust,no_run
use ferrokit::*;

let journal = DocumentType::new(&["rtf"], Vec::new, |doc: &Document<Vec<u8>>| {
    let model = TextModel::new(ViewId(1), "");
    text_editor(&model).format_bar().bind_rtf(doc.content()).accessibility_label("Entry")
})
.read(|path| std::fs::read(path))
.write(|rtf, path| std::fs::write(path, rtf));
let app = App::new("Journal").documents(journal).menu("Format", format_menu());
# let _ = app;
```

The `journal` example is a rich text document app with Export as HTML.

### Styled text and syntax highlighting

The text stays plain, but runs of it can be coloured and emphasised.
`.syntax(...)` highlights a language; `Syntax::sql()` and `Syntax::rust()`
are presets, and `Syntax::new()` builds others from keywords, strings,
comments and numbers.

```rust,no_run
use ferrokit::*;

let query = TextModel::new(ViewId(3), "SELECT * FROM orders -- all of them");
query.set_font(Some("Menlo"), 12.0);
let editor = text_editor(&query).syntax(Syntax::sql()).accessibility_label("Query");
# let _ = editor;
```

`.syntax(...)` also makes it a code editor (`.code()` does that alone):
quotes and dashes stay as typed instead of becoming smart quotes, and
there's no text replacement, autocorrection or spell checking.

For anything else, `.highlight(|text| runs)` returns UTF-8 byte ranges and
their `TextStyle`s (a `Color`, a background, bold, italic, underline).
The colours are the system's, so they follow Dark Mode. Styles are applied
after every edit, before it's drawn, and only where they changed. If the
function reads signals, it runs again when they change, here to mark a
search term:

```rust,no_run
use ferrokit::*;

let doc = TextModel::new(ViewId(4), "");
let term = Signal::new(String::from("TODO"));
let rust = Syntax::rust();
let editor = text_editor(&doc).highlight(move |text| {
    let mut runs = rust.highlight(text);
    let t = term.get();
    if !t.is_empty() {
        let mark = TextStyle::new().background(Color::Yellow);
        runs.extend(text.match_indices(t.as_str()).map(|(at, m)| (at..at + m.len(), mark)));
    }
    runs
});
# let _ = editor;
```

Styles aren't part of the text: typing, pasting and undo work on characters
only, and the highlighter restyles what changed.

## Drawing and charts

`canvas(|p| …)` draws with a `Painter`: rectangles, circles, arcs, lines,
polygons and text, in points from the top-left corner, in `Color`s
(the system's follow Dark Mode). The drawing is recorded and replayed by
a native view. It's recorded again when signals it reads change or the
canvas resizes, so a canvas is as reactive as a label.

```rust,no_run
use ferrokit::*;

let level = Signal::new(0.4);
let meter = canvas(move |p| {
    p.fill_rounded_rect(0.0, 0.0, p.width(), p.height(), 6.0, Color::Separator);
    p.fill_rounded_rect(0.0, 0.0, p.width() * level.get(), p.height(), 6.0, Color::Green);
})
.on_click(move |x, _y| println!("clicked at {x}"))          // selects, say
.on_double_click(move |x, _y| println!("opened at {x}"))   // after the click, on the second one
.description(move || format!("Level {:.0}%", level.get() * 100.0))
.min_size(200.0, 24.0);
# let _ = meter;
```

VoiceOver sees a canvas as an image. `.description(…)` is what it reads,
kept current; a canvas without one is decoration (a tinted circle behind
a symbol) and VoiceOver skips it.

`p.stroke_arc(cx, cy, r, 0.0..270.0, color, width)` draws part of a
circle with round ends, clockwise in degrees from the top: a progress ring
is a faint full circle and an arc through `0.0..360.0 * done`.

Charts are canvases. `bar_chart(|| vec![(label, value)…])` draws bars
for categories. `line_chart(|| vec![Series::new(name, points)…])` draws
lines through numeric points. Both give round-number axes and gridlines,
show values under the pointer, and describe themselves to VoiceOver ("Bar
chart, 72 bars, from … to …"). A line keeps at most a few points per
pixel column (the first, lowest, highest and last), so a million rows draw
in a few milliseconds and spikes still show. Stacked bars, areas,
scatter plots, pies, legends and formats for money, percentages and dates
are in [More controls and charts](https://docs.rs/ferrokit/latest/ferrokit/guide/ch12_more_controls_and_charts/).

```rust,no_run
use ferrokit::*;

let prices = Signal::new(vec![(1.0, 10.0), (2.0, 12.5), (3.0, 11.0)]);
let chart = line_chart(move || vec![Series::new("price", prices.get())]);
# let _ = chart;
```

## Clipboard, dragging out, dropping in

`Clipboard::general()` is the system clipboard: `set_text`, `set_png`,
`set_files`, and reading each back. Tables and text already handle ⌘C and
⌘V themselves. `Clipboard::private()` is a pasteboard of the app's own,
handy in tests.

A canvas can be dragged out. `.draggable(|| Some(item))` gives a
`DragItem`: text, a PNG, a file, or a file *promised* by name and written
where it's dropped (the Finder, Mail, another app). The write runs off the
main thread, and only if the drag lands. `.exportable("chart")` makes a
canvas an image: right-click for Copy Image and Save Image…, or drag out
`chart.png`.

```rust,no_run
use ferrokit::*;

let rows = Signal::new(vec![("a".to_string(), 1.0), ("b".to_string(), 2.0)]);
let chip = canvas(|p| p.text(4.0, 3.0, "data.csv", 11.0, Color::Secondary, Align::Start))
    .draggable(move || {
        let csv: String = rows.get_untracked().iter().map(|(k, v)| format!("{k},{v}\n")).collect();
        Some(DragItem::promise("data.csv", move |path| std::fs::write(path, &csv)))
    })
    .frame(80.0, 20.0);
let chart = bar_chart(move || rows.get()).exportable("chart");
# let _ = (chip, chart);
```

Files dropped on a window go to `Window::on_drop_files(extensions,
action)` (any file if the list is empty). In a document app, files of the
app's kinds dropped on a window open as documents, with no code.
