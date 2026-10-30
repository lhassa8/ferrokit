# Ferrokit

Native Mac apps in Rust, made from real AppKit controls: an actual `NSTableView`, `NSTextView` or `NSSplitView`, not a GPU-drawn imitation and not a web view. You get native scrolling, keyboard navigation, input methods, spell check, Dark Mode and VoiceOver because they're the system's own.

```rust,ignore
use ferrokit::*;

fn main() {
    let count = Signal::new(0);
    App::new("Counter")
        .window(Window::new("Counter").view(vstack((
            label(move || format!("Clicked {} times", count.get())),
            button("Click me", move || count.update(|n| *n += 1)),
        ))))
        .run();
}
```

## Which crate do I want?

- **Building a Mac app?** Use [`ferrokit`](crates/ferrokit), with [`cargo ferrokit`](crates/cargo-ferrokit) to make, check and bundle it. Start with the [guide](crates/ferrokit/guide/README.md).
- **Already using [Iced](https://iced.rs)?** Use [`ferrokit-iced`](crates/ferrokit-iced) to put native tables and text editors inside your Iced windows.

| Crate | What it is |
| --- | --- |
| [`ferrokit`](crates/ferrokit) | The framework: whole Mac apps where every view is a real AppKit view, updated in place by fine-grained signals. |
| [`cargo-ferrokit`](crates/cargo-ferrokit) | `cargo ferrokit new`, `run`, `check`, `bundle` and `notarize`. |
| [`ferrokit-iced`](crates/ferrokit-iced) | Real AppKit views inside Iced 0.14 windows: a table that shows a million rows at 60 fps, and a text editor with native text services. |
| [`ferrokit-core`](crates/ferrokit-core) | The native layer the other two share. You don't need to depend on it yourself. |

All four crates share one version number and are released together: use matching versions. Version **0.4.0**, an alpha: the API will change. macOS 15 or later.

## Try it

A SQLite browser built with `ferrokit` (it opens a generated sample with a million orders):

```bash
cargo run --release -p ferrokit --example sql_browser
```

Start an app of your own:

```bash
cargo install cargo-ferrokit
```

```bash
cargo ferrokit new my-app && cd my-app && cargo ferrokit run
```

Iced with real AppKit views:

```bash
cargo run --release -p ferrokit-iced --example csv_browser
```

## Earlier names

Until 0.4 the crates were called `appkit-ui` (now `ferrokit`), `iced-appkit` (now `ferrokit-iced`) and `appkit-host` (now `ferrokit-core`). The old names' last releases re-export the new crates.

## Working on Ferrokit

`scripts/check-all.sh` runs every check: build, clippy, tests, docs, and each example app's unattended self-checks. `scripts/bundle-example.sh <example>` wraps an Iced example in an `.app` bundle.

## License

MIT OR Apache-2.0, at your option.
