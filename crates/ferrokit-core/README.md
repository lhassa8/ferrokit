# ferrokit-core

**You probably want [`ferrokit`](https://crates.io/crates/ferrokit)** (native Mac apps). This crate is the layer it's built on.

The native layer under [`ferrokit`](https://crates.io/crates/ferrokit): it places real AppKit views inside a window whose content a GPU framework draws, and keeps them in step with that framework's layout. It needs only the window's `raw-window-handle` and layout rectangles in logical points.

- **Placement:** `attach`, `mount`/`place`/`unmount`, clip containers for scroll areas, pixel snapping, per-frame lifecycle (`begin_frame`/`end_frame`).
- **Events:** one sink for Tab focus steps, focus changes, control actions, appearance changes and redraw requests.
- **Keyboard:** Tab order across both worlds, and edit shortcuts routed to whichever side has focus (with an optional Edit menu).
- **Overlays:** snapshots of native views, for drawing host overlays above them.
- **Widgets:** `table::TableModel` (`NSTableView`) and `text::TextModel` (`NSTextView`).

Use it directly only to add support for another host framework. Part of [Ferrokit](https://github.com/lhassa8/ferrokit); released together with the other Ferrokit crates under one version number. macOS 15 or later. Main thread only.

License: MIT OR Apache-2.0.
