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
use appkit_ui::*;

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

### Groups, icons, progress bars, fitted columns

```rust,no_run
use std::rc::Rc;
use appkit_ui::*;

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

## Outlines and the sidebar

`outline(roots, title)` shows a tree of `Identifiable` values; expansion and
selection survive updates. `sidebar(…)` is the same in the source-list
style of Mail's or Finder's sidebar.

```rust,no_run
use appkit_ui::*;

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
        vec![TreeNode::leaf(Folder { id: 2, name: "appkit-ui".into() })],
    )]
};
let view = sidebar(tree, |f: &Folder| f.name.clone()).icon(|_| Some("folder".into())).selection(&selected);
# let _ = view;
```

## Collections: grids of items

`collection(items, describe)` shows items in a grid (`NSCollectionView`),
as Finder's icon view or Photos do. Only the items on screen have views:
cells are made once and refilled as they scroll in, so 100,000 items load,
lay out and draw in about 5 ms.

```rust,no_run
use std::path::PathBuf;
use appkit_ui::*;

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
use appkit_ui::*;

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
your app's own undoable changes (see [The app shell](06-app-shell.md)).

### Styled text and syntax highlighting

The text stays plain, but runs of it can be coloured and emphasised.
`.syntax(...)` highlights a language; `Syntax::sql()` and `Syntax::rust()`
are presets, and `Syntax::new()` builds others from keywords, strings,
comments and numbers.

```rust,no_run
use appkit_ui::*;

let query = TextModel::new(ViewId(3), "SELECT * FROM orders -- all of them");
query.set_font(Some("Menlo"), 12.0);
let editor = text_editor(&query).syntax(Syntax::sql()).accessibility_label("Query");
# let _ = editor;
```

For anything else, `.highlight(|text| runs)` returns UTF-8 byte ranges and
their `TextStyle`s (a `TextColor`, a background, bold, italic, underline).
The colours are the system's, so they follow Dark Mode. Styles are applied
after every edit, before it's drawn, and only where they changed. If the
function reads signals, it runs again when they change, here to mark a
search term:

```rust,no_run
use appkit_ui::*;

let doc = TextModel::new(ViewId(4), "");
let term = Signal::new(String::from("TODO"));
let rust = Syntax::rust();
let editor = text_editor(&doc).highlight(move |text| {
    let mut runs = rust.highlight(text);
    let t = term.get();
    if !t.is_empty() {
        let mark = TextStyle::new().background(TextColor::Yellow);
        runs.extend(text.match_indices(t.as_str()).map(|(at, m)| (at..at + m.len(), mark)));
    }
    runs
});
# let _ = editor;
```

Styles aren't part of the text: typing, pasting and undo work on characters
only, and the highlighter restyles what changed.

## Drawing and charts

`canvas(|p| …)` draws with a `Painter`: rectangles, circles, lines,
polygons and text, in points from the top-left corner, in `TextColor`s
(the system's follow Dark Mode). The drawing is recorded and replayed by
a native view. It's recorded again when signals it reads change or the
canvas resizes, so a canvas is as reactive as a label.

```rust,no_run
use appkit_ui::*;

let level = Signal::new(0.4);
let meter = canvas(move |p| {
    p.fill_rounded_rect(0.0, 0.0, p.width(), p.height(), 6.0, TextColor::Separator);
    p.fill_rounded_rect(0.0, 0.0, p.width() * level.get(), p.height(), 6.0, TextColor::Green);
})
.on_click(move |x, _y| println!("clicked at {x}"))
.description(move || format!("Level {:.0}%", level.get() * 100.0))
.min_size(200.0, 24.0);
# let _ = meter;
```

VoiceOver sees a canvas as an image. `.description(…)` is what it reads,
kept current.

Charts are canvases. `bar_chart(|| vec![(label, value)…])` draws bars
for categories. `line_chart(|| vec![Series::new(name, points)…])` draws
lines through numeric points. Both give round-number axes and gridlines,
show values under the pointer, and describe themselves to VoiceOver ("Bar
chart, 72 bars, from … to …"). A line keeps at most a few points per
pixel column (the first, lowest, highest and last), so a million rows draw
in a few milliseconds and spikes still show.

```rust,no_run
use appkit_ui::*;

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
use appkit_ui::*;

let rows = Signal::new(vec![("a".to_string(), 1.0), ("b".to_string(), 2.0)]);
let chip = canvas(|p| p.text(4.0, 3.0, "data.csv", 11.0, TextColor::Secondary, Align::Start))
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
