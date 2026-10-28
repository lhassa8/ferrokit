# Changelog

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
