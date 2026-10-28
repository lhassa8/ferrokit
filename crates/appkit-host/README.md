# appkit-host

The host-agnostic layer under [`iced-appkit`](https://crates.io/crates/iced-appkit): it places real AppKit views inside a window whose content a GPU framework draws, and keeps them in step with that framework's layout. It needs only the window's `raw-window-handle` and layout rectangles in logical points.

- **Placement:** `attach`, `mount`/`place`/`unmount`, clip containers for scroll areas, pixel snapping, per-frame lifecycle (`begin_frame`/`end_frame`).
- **Events:** one sink for Tab focus steps, focus changes, control actions, appearance changes and redraw requests.
- **Keyboard:** Tab order across both worlds, and edit shortcuts routed to whichever side has focus (with an optional Edit menu).
- **Overlays:** snapshots of native views, for drawing host overlays above them.
- **Widgets:** `table::TableModel` (`NSTableView`) and `text::TextModel` (`NSTextView`).

Use it directly only to add support for another host framework; Iced apps use `iced-appkit`. macOS 15 or later. Main thread only.

License: MIT OR Apache-2.0.
