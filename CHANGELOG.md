# Changelog

## appkit-ui 0.3.0, appkit-host 0.2.2, cargo-ferrokit 0.1.0 (unreleased: versions set, not yet published)

`appkit-ui` 0.3 makes signals `Copy`, adds async data, commands, styled text, popovers, inspectors, a canvas with charts, the clipboard and drag and drop, and localisation. Its flagship example is a SQLite browser. `cargo-ferrokit` is new: it makes, runs, checks, bundles and notarizes apps. `appkit-host` only gains API (a patch release). `iced-appkit` is unchanged and stays at 0.2.1, whose `appkit-host` requirement 0.2.2 satisfies.

### appkit-ui 0.3.0

Changed (breaking; `cargo semver-checks`: `Signal` and `Memo` now implement `Copy`):

- **`Copy` signals:** `Signal` and `Memo` are `Copy` handles into a per-thread arena, so closures capture them without `.clone()`. Existing code still compiles; clippy's `clone_on_copy` points out the clones left.
  - Each signal belongs to the owner current when it was made, and is freed with it: a window's view tree (including signals made by its buttons' actions), a `dynamic` branch, a `for_each` row, or a document. Signals made outside any owner (in `main`) live for the app.
  - Reading a signal after its owner is gone panics with a message saying so; setting it does nothing. New: `Signal::try_get`, `Signal::is_alive`, `live_signals()` for leak checks. `Signal` compares by identity.

Added:

- **Async data:** `Resource::new(source, fetch)` loads on a background thread from the signals `source` reads, and again when they change.
  - A new load cancels the running one (`Cancel::is_cancelled`), so stale results are never shown, and the last value stays while the next loads.
  - `get`, `get_untracked`, `loading`, `error`, `state` (`Load`), `refetch`. Closing its window cancels the load.
- **Commands and focus:** `Command::new(name)`, handled by views (`ViewExt::on_command`, `on_command_when`), windows (`Window::on_command`) and the app (`App::on_command`).
  - `menu_command` and `toolbar_command` run the handler nearest the focused view, and are enabled only while one answers.
  - `ViewExt::focused_value` / `focused_value::<T>()`; `perform`, `can_perform`, `perform_in`, `focused_window`; `MenuItem::title_when`.
- **Styled text and syntax highlighting:** `TextEditor::highlight(|text| runs)` (`TextStyle`: a `TextColor`, background, bold, italic, underline).
  - It's applied after every edit, before it's drawn, restyling only the span that changed, and again when signals it reads change.
  - `TextEditor::syntax(Syntax::sql() / Syntax::rust() / Syntax::new()…)`.
- **Window furniture:**
  - `Split::stacked()` and `Split::collapsible(index, &signal)` for sidebars and inspectors: they hide with a signal or a drag, keep their size, and reopen at it. `Split::trailing(points)`.
  - `ViewExt::popover(&shown, || content)` / `popover_on(Edge, …)`: a native popover; Escape or a click outside closes it, and its content is freed.
  - `DocumentType::toolbar`.
- **Drawing and charts:** `canvas(|p: &mut Painter| …)`, redrawn when the signals it reads change or it resizes; `on_click`, `on_hover`, a VoiceOver `description`.
  - `bar_chart` and `line_chart` (`Series`): round-number axes, values under the pointer, a VoiceOver summary, and decimation (a million points draw as a few thousand).
- **Clipboard, drag and drop:**
  - `Clipboard` (general or private): text, PNG, files.
  - `DragItem`: text, PNG, a file, or a promised file written where it's dropped. `Canvas::draggable`, `Canvas::exportable` (Copy Image, Save Image…, drag out a PNG).
  - `Window::on_drop_files(extensions, action)`; document apps open dropped files of their kinds.
- **Localisation:** `t` (English keys) and `App::translations(&[(lang, .strings)])`, in the user's first preferred language the app has.
  - `t_with` (named placeholders), `t_plural` (CLDR plural rules), `format_number`, `set_language`.
  - The framework's own strings in German, French and Japanese.
- **Test helpers:** `testing::canvas_drawing`, `canvas_png`, `canvas_drag`, `fulfil`, `drop_on_view`.
- **New example, `sql_browser`:** a SQLite document app.
  - Schema sidebar; a highlighted SQL editor over a million-row results table; queries that cancel stale ones.
  - Row inspector, query history, charts, CSV export (drag `results.csv` to the Finder), German.

Fixed:

- A collapsible pane no longer collapses itself at the first layout.
- Popovers reopen after an animated close in an app that isn't active.

### appkit-host 0.2.2 (additive; semver-checks: no update required)

- `TableModel::set_columns(columns)`: new columns in the same native table (the sort is cleared). Text cells set their alignment on reuse too.
- `TextModel::set_highlighter`, `restyle`, `style_at`, with `TextStyle` and `TextColor` (system colours that follow Dark Mode, including `Background` and `Separator`; `TextColor::ns()`). Styled runs over plain text are applied before an edit is drawn and kept out of undo.
- `testing::drop_on_view`.

### cargo-ferrokit 0.1.0 (new)

- `cargo ferrokit new <path> [--document]`: a working app (a to-do list, or a text document app) with self-checks, on `appkit-ui` 0.3 (or a local checkout: `--appkit-ui-path`).
- `run`; `check` (clippy with warnings as errors, tests, a bundle, the app's `CHECK=all` self-checks, the signature); `bundle` (signed `.app`, hardened runtime, icon from a PNG, document types, languages, `--universal`, `--zip`); `notarize` (Developer ID, notary service, stapling; `--dry-run`).
- Settings in `[package.metadata.ferrokit]`.

## appkit-host 0.2.1, iced-appkit 0.2.1, appkit-ui 0.2.0 (2026-10-29)

`appkit-host` and `iced-appkit` only gain API (patch releases); `iced-appkit` re-exports the new `CellImage`, so it needs `appkit-host` 0.2.1. `appkit-ui` moves to 0.2.0: besides new features, some behaviour changed (see below).

### appkit-host 0.2.1 (additive)

- Group rows: `TableModel::set_group_by(key)` puts a header before each group (full width, floating, not selectable; sorting sorts within groups); `groups()`, `shown_rows()`. `row_at` is `None` for a header; event indices stay model indices.
- `Column::progress` (a bar from 0 to 1), `Column::icon` (`CellImage::Symbol` or `FileIcon` before the text), `Column::fit` (width from the longest values).

### iced-appkit 0.2.1 (additive)

- Re-exports `CellImage` (for `Column::icon`); requires `appkit-host` 0.2.1.

### appkit-ui 0.2.0

Changed behaviour:

- `image(symbol)` is decorative (hidden from VoiceOver) unless given a `.description`; it used to read the symbol name.
- `quit()` starts on the next run-loop pass (it used to terminate inside the caller's batch).
- Windows closed by the close button or ⌘W are torn down (views, effects, undo history), like `close_window`; the Settings window is kept.
- `layout_times_us()` keeps the most recent 10,000–20,000 passes (it grew forever); `layout_passes()` gives the total.

Added:

- **Document apps:** `App::documents(DocumentType)`.
  - The File menu gets New, Open…, Open Recent, Save, Save As… and Revert to Saved.
  - Each window is a `Document<T>`: a content signal and a file. The title bar shows the edited dot and the file's proxy icon.
  - Closing or quitting with changes asks to save.
  - `new_document`, `open_document`, `save_document(_as)`, `revert_document`, `document_edited`, `recent_documents`.
- Document apps open files from the Finder (`application:openURLs:`): double-click, Open With, drops on the Dock icon; the untouched Untitled window from launch then closes. `package-app.sh --doc-type "Name:ext,ext"` declares the types.
- `dynamic_submenu(title, items)`: a submenu rebuilt each time it opens.
- `collection(items, describe)`: a grid of items (`NSCollectionView`), virtualised (cells reused), with a title, subtitle and an image (SF Symbol, file icon or colour) per item; two-way selection by id (single or multiple); double-click and Return activate; VoiceOver reads items by title. `collection_cells_made()` diagnostic.
- More controls, each bound two ways to a signal: `date_picker` (`SystemTime`; with time, calendar style, range), `color_well` (`Color`), `segmented`, `combo_box` (text plus suggestions), `search_field` (placeholder, submit, clear).
- `testing::ax_audit` also requires names on date pickers and colour wells, and skips the parts AppKit draws inside a named date picker.
- `quit()` now starts on the next run-loop pass, so prompts shown while quitting don't run inside the caller's batch.
- Fix: a scroll view re-fits its content whenever AppKit changes the visible width (classic scroll bars appearing while drawing).
- Fix: with classic (always-shown) scroll bars, as on a Mac with a mouse, a `scroll` view's content stayed wider than the view once the vertical bar appeared (a stray horizontal scroll bar, clipped content). The content now re-fits.
- `testing::heap_in_use_mb()`: live allocations, for leak checks (footprint lags while the allocator returns freed pages).
- `testing::capture_window` returns `None` for a capture without the window's content (no screen-recording permission).

## 0.2.0 (2026-10-29)

`appkit-host` 0.2.0 has breaking changes, and `iced-appkit` re-exports its types (`host`, `TableEvent`, `Column`…), so both move to 0.2.0. `appkit-ui` 0.1.0 is new.

### appkit-host 0.2.0 (breaking)

- `TableEvent` is now `#[non_exhaustive]` and has a new `Edit { row, column, value }` variant.
- `Column::editable(true)` makes a text or number column's cells editable. Finishing an edit emits `TableEvent::Edit`; apply it with `update_row`.
- `TableModel::scroll_view()` returns the table's scroll view, for hosts that place it themselves.
- `observe_appearance()` is public, for apps without an attached host window.
- `TextModel::scroll_view()`.
- `TableModel::append(rows)` adds rows for live data: when unsorted, its cost doesn't depend on the rows already there, and the selection is kept. `scroll_to_end()` and `is_at_end()` support following a tail.
- Tables copy: ⌘C copies the selected rows as tab-separated text (columns in their shown order). `TableModel::selected_text()`, `copy_to(&pasteboard)`.
- Opt-in table transfers, each reported as a new `TableEvent` that the app applies:
  - `set_row_drag(true)`: rows drag out as text.
  - `set_reorderable(true)`: `RowsMoved { rows, before }`; apply with the new `move_rows`. Refused while sorted.
  - `set_file_drop(Some(&["csv"]))`: `FilesDropped { paths, before }`.
  - `set_pastes(true)`: ⌘V gives `Paste(text)`; `paste_from(&pasteboard)`.
- `testing`: `drag_rows`, `drop_on_table`, `files_pasteboard`, and private-pasteboard helpers, for checking drags without a mouse or the clipboard.
- `TextModel::share_undo(Some(&manager))` records the editor's edits in another undo manager (a window's, shared with other views); `None` returns to a history of its own, still the default. `TextModel::undo_manager()` returns the one in use. `set_text` drops only this editor's steps from a shared history.
- Fix: `TextEvent::Changed` is now sent for undo and redo too. AppKit sends no `textDidChange:` for them, so the model also listens to its text storage; a queued `Changed` isn't repeated.

### iced-appkit 0.2.0

- `table(..).on_edit(..)` for editable columns.
- Depends on `appkit-host` 0.2 (its re-exported `TableEvent` gained variants and is now `#[non_exhaustive]`).

### appkit-ui 0.1.0 (new)

A native macOS UI framework on `appkit-host`: real AppKit views built once and updated in place by fine-grained signals.

- Signals, memos, effects, batching, background work (`spawn`), stored settings.
- Layout in Rust: stacks, grids, spacers, padding and sizes, keyed lists (`for_each`), `dynamic` subtrees, subtree-only relayout.
- Controls bound to signals: text and secure fields, toggles, checkboxes, pickers, sliders, steppers, progress, SF Symbols, buttons; `.accessibility_label`.
- Deep widgets: `table` (sorting, editing, copy, drag and drop), `outline` and `sidebar`, `text_editor`.
- App shell: menus with shortcuts and state, toolbars, split views, sheets, alerts, open and save panels, Settings, window and split restoration, per-window undo, `on_frame` for live data; windows torn down when closed.
- `testing`: self-check helpers, accessibility audits, window captures.
- A guide (`guide/`) whose examples compile as doc tests; example apps `settings`, `native_csv`, `log_viewer`, `hands_on`.

## 0.1.0 (2026-10-28)

First release of `iced-appkit` and `appkit-host`: real AppKit controls inside Iced 0.14 windows on macOS 15+.

- **Embedding core:**
  - `NativeSlot` places a native view in the Iced layout, clipped inside scrollables and snapped to the pixel grid.
  - `root()` keeps native views in step with Iced's frames, and swaps them for pictures of themselves while any Iced overlay (dropdown, tooltip, menu) covers them. `cover()` does the same for modal layers.
  - Native views join Iced's Tab order, wrapping at the ends. Clicks move focus across the boundary.
  - Edit shortcuts and an optional Edit menu go to the focused side.
  - `theme::system()` follows the system colours, accent colour and Dark Mode.
- **Table:** `TableModel<T>` + `table()`.
  - Lazy rows (1,000,000 rows at 60 fps with flat memory).
  - Text, number and checkbox columns; resizable and reorderable columns with saved state.
  - Natural-order sorting in Rust (100,000 rows in ~70 ms). The selection is kept across sorts, inserts and removes.
  - `on_select`, `on_activate` (double-click/Return), `on_sort`, `on_toggle`.
  - Type-select and full VoiceOver structure.
- **Text editor:** `TextModel` + `text_editor()`.
  - Plain text with IME composition events (`on_composition`), and `on_change` held back while composing.
  - Spell and grammar checking with suggestions, Look Up, Translate, find bar, fonts, insets, read-only mode.
  - Undo kept per editor. Tab inserts a tab; Ctrl-Tab moves on.
  - UTF-8 byte offsets in the API.
- **Examples:** `csv_browser` (demo) and `checklist` (regression suite, `CHECK=all`).

Known limits:
- One window per app.
- An overlay appears ~2 frames after opening, while the native picture is prepared.
- Native views are hidden from accessibility while an overlay covers them.
- Iced's own widgets aren't exposed to VoiceOver.
- No rich text or editable table cells yet.
