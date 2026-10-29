# appkit-ui

A native macOS UI framework for Rust. Every view is a real AppKit view (an
actual `NSTableView`, `NSTextView`, `NSSplitView`), built once and updated
in place by fine-grained signals. No virtual DOM, no diffing, no web view,
no custom drawing.

```rust,no_run
use appkit_ui::*;

fn main() {
    let count = Signal::new(0);
    let (c1, c2) = (count.clone(), count.clone());
    App::new("Counter")
        .window(Window::new("Counter").size(320.0, 140.0).view(
            vstack((
                label(move || format!("Clicked {} times", c1.get())),
                button("Click me", move || c2.update(|n| *n += 1)),
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
- **The app shell:** menus with shortcuts and state, toolbars, split views,
  sheets, alerts, open and save panels, a Settings window, stored settings,
  window restoration, per-window undo.
- **Live data:** `on_frame` applies incoming data once per display frame;
  the log viewer example follows a growing 1,000,000-line file at 60 Hz.
- **Accessibility:** VoiceOver reads AppKit's own accessibility tree;
  `.accessibility_label` names controls, and `testing::ax_audit` finds the
  ones without a name.
- **Testing and shipping:** self-check helpers, plus scripts to package a
  signed, notarised `.app`.

Read the [guide](guide/README.md); every example in it is compiled as a doc
test. The examples (`settings`, `native_csv`, `log_viewer`, `hands_on`) are
complete apps:

```bash
cargo run --release -p appkit-ui --example native_csv
```

macOS 15 or later. Main thread only. Built on
[`appkit-host`](https://crates.io/crates/appkit-host) and objc2.

**Status:** 0.2.0, an alpha: the API will change. Part of [Ferrokit](https://github.com/lhassa8/ferrokit).

License: MIT OR Apache-2.0.
