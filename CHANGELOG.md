# Changelog

## Unreleased

### appkit-host (breaking: release as 0.2.0)

- `TableEvent` is now `#[non_exhaustive]` and has a new `Edit { row, column, value }` variant.
- `Column::editable(true)` makes a text or number column's cells editable. Finishing an edit emits `TableEvent::Edit`; apply it with `update_row`.
- `TableModel::scroll_view()` returns the table's scroll view, for hosts that place it themselves.
- `observe_appearance()` is public, for apps without an attached host window.
- `TextModel::scroll_view()`.
- Fix: `TextEvent::Changed` is now sent for undo and redo too. AppKit sends no `textDidChange:` for them, so the model also listens to its text storage; a queued `Changed` isn't repeated.

### iced-appkit

- `table(..).on_edit(..)` for editable columns.

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
