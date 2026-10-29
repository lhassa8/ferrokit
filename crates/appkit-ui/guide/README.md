# The appkit-ui guide

`appkit-ui` builds Mac apps in Rust from real AppKit views: an actual
`NSTableView`, `NSTextView`, `NSSplitView` and so on. You describe the views
once, connect them to signals, and each native view updates in place when
the state it reads changes. There is no virtual DOM and no diffing.

Every Rust example in these chapters is compiled by `cargo test --doc`, so
the guide can't drift from the API.

1. [Getting started](01-getting-started.md): an app, a window, a button.
2. [State](02-state.md): signals, memos, effects, batching, background work.
3. [Views and layout](03-views-and-layout.md): stacks, grids, modifiers,
   lists and identity.
4. [Controls and forms](04-controls-and-forms.md): fields, toggles,
   pickers, sliders, and naming them for VoiceOver.
5. [Tables, outlines and editors](05-tables-outlines-editors.md): the deep
   widgets.
6. [The app shell](06-app-shell.md): menus, toolbars, split views, sheets,
   panels, Settings, restoration and undo.
7. [Live data](07-live-data.md): per-frame updates and growing tables.
8. [Testing and shipping](08-testing-and-shipping.md): self-checks,
   accessibility audits, packaging and notarisation.

The example apps in `crates/appkit-ui/examples` are the long form of these
chapters:

| Example | Shows |
| --- | --- |
| `settings` | a settings form: grid layout, validation, background work |
| `native_csv` | a 200,000-row CSV browser: table, sidebar, search, sheet, notes |
| `log_viewer` | a log tail at display rate: `on_frame`, `append`, filtering |
| `hands_on` | drag and drop, the clipboard, panels, undo, input methods |
| `ui_checklist` | the framework's regression suite |

Run one with `cargo run --release -p appkit-ui --example settings`.
