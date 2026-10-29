# Changelog

## Unreleased

### appkit-ui

- **Document apps:** `App::documents(DocumentType)`.
  - The File menu gets New, Open…, Open Recent, Save, Save As… and Revert to Saved.
  - Each window is a `Document<T>`: a content signal and a file. The title bar shows the edited dot and the file's proxy icon.
  - Closing or quitting with changes asks to save.
  - `new_document`, `open_document`, `save_document(_as)`, `revert_document`, `document_edited`, `recent_documents`.
- `dynamic_submenu(title, items)`: a submenu rebuilt each time it opens.
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
