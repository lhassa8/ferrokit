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

Use [`ferrokit`](crates/ferrokit), with [`cargo ferrokit`](crates/cargo-ferrokit) to make, check and bundle your app. Start with the [guide](crates/ferrokit/guide/README.md).

| Crate | What it is |
| --- | --- |
| [`ferrokit`](crates/ferrokit) | The framework: whole Mac apps where every view is a real AppKit view, updated in place by fine-grained signals. |
| [`cargo-ferrokit`](crates/cargo-ferrokit) | `cargo ferrokit new`, `run`, `check`, `bundle` and `notarize`. |
| [`ferrokit-core`](crates/ferrokit-core) | The AppKit layer under `ferrokit`. You don't need to depend on it yourself. |

The crates share one version number and are released together: use matching versions. Version **0.4.0**, an alpha: the API will change. macOS 15 or later.

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

## Earlier names

Until 0.4 the crates were called `appkit-ui` (now `ferrokit`) and `appkit-host` (now `ferrokit-core`); the old names' last releases re-export the new crates. `iced-appkit`, real AppKit views inside Iced windows, stays at 0.2.1 and isn't continued: Ferrokit is one framework now.

## Working on Ferrokit

`scripts/check-all.sh` runs every check: build, clippy, tests, docs, and each example app's unattended self-checks.

## License

MIT OR Apache-2.0, at your option.
