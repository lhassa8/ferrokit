# ferrokit-core

**You probably want [`ferrokit`](https://crates.io/crates/ferrokit)** (native Mac apps). This crate is the layer it's built on.

The AppKit layer under [`ferrokit`](https://crates.io/crates/ferrokit): it places real AppKit views in a window, keeps their frames, focus and keyboard in step with `ferrokit`'s own layout, and holds the models for its deepest widgets.

- **Placement:** `attach`, `mount`/`place`/`unmount`, clip containers for scroll areas, pixel snapping, per-frame lifecycle (`begin_frame`/`end_frame`).
- **Events:** one sink for Tab focus steps, focus changes, control actions, appearance changes and redraw requests.
- **Keyboard:** Tab order and edit shortcuts (with an optional Edit menu).
- **Widgets:** `table::TableModel` (`NSTableView`) and `text::TextModel` (`NSTextView`). `ferrokit` re-exports them, and their methods are documented here.

Depend on `ferrokit` instead; it pins this crate's version exactly. Part of [Ferrokit](https://github.com/lhassa8/ferrokit); released together with the other Ferrokit crates under one version number. macOS 15 or later. Main thread only.

License: MIT OR Apache-2.0.
