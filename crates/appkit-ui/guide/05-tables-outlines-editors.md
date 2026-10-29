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
let f = files.clone();
let grid = collection(move || f.get(), |file: &File| {
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
