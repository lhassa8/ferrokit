# Wrapping any NSView in a Rust UI

Ferrokit wraps a lot of AppKit, but not every view, and not the frameworks beside it: MapKit's map, SceneKit, PDFKit's thumbnails, a third-party view, or a view your app draws itself. `native(..)` takes any `NSView` you make with the `objc2` bindings and makes it a Ferrokit view: it's laid out with the others, updates from signals, takes part in focus and VoiceOver, and is torn down with its window.

```rust,no_run
{{#include ../snippets/src/bin/level_meter.rs}}
```

The `objc2` crates go in your `Cargo.toml`, with the features for the classes you call:

```toml
[dependencies]
ferrokit = "0.8"
objc2-app-kit = { version = "0.3", features = ["NSResponder", "NSView", "NSControl", "NSLevelIndicator"] }
```

- `native(make)` calls `make(mtm)` once, on the main thread, when the window is built.
- `.update(f)` runs `f` as an effect: now, and again whenever a signal it read changes. Give unrelated properties their own `update`s.
- `.measure(..)` and `.flexible(..)` size it; `.keep(delegate)` keeps a delegate alive (AppKit holds delegates weakly); `.on_remove(..)` cleans up.
- A wrapped view is a leaf: Ferrokit views can't go inside it.

The `native_views` example wraps `MKMapView` and a clock face the app draws itself:

```bash
git clone https://github.com/lhassa8/ferrokit && cd ferrokit && cargo run -p ferrokit --example native_views
```

[Custom native views](guide/14-custom-native-views.md) has delegates, drawing, focus and accessibility.
