# A real NSTableView from Rust: a million rows

You can drive AppKit's `NSTableView` from Rust directly with the `objc2-app-kit` bindings: implement `NSTableViewDataSource` and `NSTableViewDelegate` with `define_class!`, make cell views, keep the delegate alive, and handle sorting and selection yourself. That works, and it's the right choice for one table in an otherwise non-AppKit app.

Ferrokit does that part for you. Its `table` is a real `NSTableView`, virtualised (only the rows on screen have views), fed from a model of your own Rust rows:

```rust,no_run
{{#include ../snippets/src/bin/million_rows.rs}}
```

What you get without writing it:

- **Scrolling and keyboard** are the table's own: arrow keys, Page Up and Down, type-to-select, ⌘A, ⌘-click and ⇧-click.
- **Sorting:** clicking a header sorts by that column (numbers as numbers, dates as dates). Row numbers in every event stay *model* indices, positions in what you gave `set_rows`, whatever the sort.
- **A million rows** load fast: 200,000 in about 150 ms. `set_rows` replaces the rows in place; `update_row`, `push`, `insert` and `remove` change single rows.
- **Copy and drag:** ⌘C copies the selected rows as tab-separated text that Numbers and Excel paste as cells; `.row_drag()` drags rows out.
- **VoiceOver** reads it as a table, row by row and cell by cell.

More: editable cells, date and progress columns, icons, grouped rows with floating headers, a focused cell as in a spreadsheet, row menus, drag to reorder, and files dropped from the Finder. All in [Tables, outlines and editors](guide/05-tables-outlines-editors.md), with `NSOutlineView` for trees and `NSBrowser` for column views.

To see a million rows for yourself:

```bash
git clone https://github.com/lhassa8/ferrokit && cd ferrokit && cargo run --release -p ferrokit --example sql_browser
```

## When to use `objc2` directly instead

- Your app's UI is drawn by something else (egui, Iced, a game engine) and you need one AppKit view inside it: Ferrokit runs the whole window, so it doesn't fit there.
- You need an `NSTableView` feature Ferrokit doesn't expose. Then you can still use Ferrokit and reach for the bindings for that one part: [`native(..)`](wrap-any-nsview.md) wraps any `NSView`.
