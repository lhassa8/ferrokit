# Ferrokit

Real AppKit controls for Rust apps on macOS. Not GPU-drawn imitations and not web views: an actual `NSTableView` or `NSTextView`, so you get native scrolling, keyboard navigation, IME, spell check, Dark Mode and VoiceOver for free.

| Crate | What it is |
| --- | --- |
| [`iced-appkit`](crates/iced-appkit) | Real AppKit views inside [Iced](https://iced.rs) 0.14 windows: a table that shows a million rows at 60 fps, and a plain-text editor with native text services. |
| [`appkit-host`](crates/appkit-host) | The framework-agnostic layer underneath: places AppKit views inside a GPU-rendered window and keeps layout, clipping, focus, shortcuts and appearance in step. |

Status: **0.1.0**, macOS 15 or later. A standalone native framework built on `appkit-host` is planned.

## Try it

```bash
cargo run --release -p iced-appkit --example csv_browser
```

`scripts/bundle-example.sh <example>` wraps an example in an `.app` bundle. The `checklist` example is the regression suite: every behaviour is on its Tests menu, and `CHECK=all` runs the unattended checks.

## License

MIT OR Apache-2.0, at your option.
