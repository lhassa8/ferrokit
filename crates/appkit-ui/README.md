# appkit-ui

A native macOS UI framework for Rust. Every view is a real AppKit view (an
actual `NSTableView`, `NSTextView`, `NSSplitView`), built once and updated
in place by fine-grained signals. No virtual DOM, no diffing, no web view,
no custom drawing.

```rust,no_run
use appkit_ui::*;

fn main() {
    let count = Signal::new(0);
    App::new("Counter")
        .window(Window::new("Counter").size(320.0, 140.0).view(
            vstack((
                label(move || format!("Clicked {} times", count.get())),
                button("Click me", move || count.update(|n| *n += 1)),
            ))
            .padding(20.0),
        ))
        .run();
}
```

It's for data-heavy desktop apps where the Mac's own widgets matter:

- **Deep widgets:** a virtualised table (200,000 rows load in about 150 ms
  and scroll at display rate; sorting, editing, drag and drop, copy and
  paste), outlines and source-list sidebars, and a text editor with spell
  checking, input methods, Find and undo.
- **The app shell:** menus with shortcuts and state, commands that act on
  the focused view, toolbars, split views with collapsible sidebars and
  inspectors, popovers, sheets, alerts, open and save panels, document
  apps, a Settings window, window restoration, per-window undo.
- **State and data:** `Copy` signals owned by the views that made them, and
  `Resource` for loading off the main thread (a newer load cancels the
  stale one).
- **Text, drawing, transfer:** syntax highlighting in the editor, a canvas
  with bar and line charts (a million points draw quickly), the clipboard,
  drag and drop (files written where they're dropped).
- **Languages:** `t("…")` with `.strings` tables and plural rules; the
  framework's own strings come in German, French and Japanese.
- **Live data:** `on_frame` applies incoming data once per display frame;
  the log viewer example follows a growing 1,000,000-line file at 60 Hz.
- **Accessibility:** VoiceOver reads AppKit's own accessibility tree;
  `.accessibility_label` names controls, and `testing::ax_audit` finds the
  ones without a name.
- **Testing and shipping:** self-check helpers, and
  [`cargo ferrokit`](https://crates.io/crates/cargo-ferrokit) to make, check,
  bundle and notarise apps.

Read the [guide](guide/README.md); every example in it is compiled as a doc
test. The examples are complete apps; the flagship is a SQLite browser
(schema sidebar, highlighted SQL, a million-row results table, charts, CSV
export):

```bash
cargo run --release -p appkit-ui --example sql_browser
```

```bash
cargo install cargo-ferrokit && cargo ferrokit new my-app
```

macOS 15 or later. Main thread only. Built on
[`appkit-host`](https://crates.io/crates/appkit-host) and objc2.

**Status:** 0.3.0, an alpha: the API will change. Part of [Ferrokit](https://github.com/lhassa8/ferrokit).

License: MIT OR Apache-2.0.
