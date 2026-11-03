# Ferrokit

Native Mac apps in Rust, made from real AppKit controls: an actual `NSTableView`, `NSTextView` or `NSSplitView`, not a GPU-drawn imitation and not a web view. You get native scrolling, keyboard navigation, input methods, spell check, Dark Mode and VoiceOver because they're the system's own.

![The SQL browser example: a schema sidebar, a highlighted query, its results table, a bar chart of them and an inspector](assets/sql-browser.png)

<p>
  <img src="assets/habits.png" width="49%" alt="The habits example: today's habits with streaks and a progress ring">
  <img src="assets/charts-dark.png" width="49%" alt="The components example in Dark Mode: a donut chart and stacked bars against a budget line">
</p>

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

## Stability

Ferrokit is young (an alpha) and has no users yet; this is what it promises meanwhile.

- **Versions:** while it's 0.x, a minor release (0.4 → 0.5) may change the API; a patch release (0.4.0 → 0.4.1) never does. Every change that breaks code is listed in the CHANGELOG with what to write instead, and a renamed item keeps working, deprecated, for one minor release where it can.
- **One version:** the crates are released together under one version; `ferrokit` pins `ferrokit-core` exactly.
- **macOS:** 15 or later. Checked on macOS 15 and the current release (CI), and on the newest beta locally. Each macOS release is checked when it ships; AppKit changes (macOS 27 turned column browser cells into text cells) are fixed in a patch.
- **Rust:** the latest stable; the minimum (`rust-version`, 1.88) only rises in a minor release.
- **Failures:** an Objective-C exception or a panic in code AppKit calls is caught where it came in, reported (`App::on_error`) and survived, never silently dropped.
- **Checks:** every change runs about 460 self-checks across 17 example apps and the project templates (`scripts/check-all.sh`), as release and as debug builds, and in the App Sandbox.

## Earlier names

Until 0.4 the crates were called `appkit-ui` (now `ferrokit`) and `appkit-host` (now `ferrokit-core`); the old names' last releases re-export the new crates. `iced-appkit`, real AppKit views inside Iced windows, stays at 0.2.1 and isn't continued: Ferrokit is one framework now.

## Working on Ferrokit

`scripts/check-all.sh` runs every check: build, clippy, tests, docs, and each example app's unattended self-checks.

## License

MIT OR Apache-2.0, at your option.
