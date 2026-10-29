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
