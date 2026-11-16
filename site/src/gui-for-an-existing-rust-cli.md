# A native Mac GUI for an existing Rust CLI

<!-- description: How to add a native macOS app to a Rust command-line tool or library you already have: a second binary behind a feature, a tokio runtime for async code, and a window that follows the CLI's data file. -->

**Short answer:** add the app as a second binary in the same package, behind a `gui` feature, so the command-line tool's build doesn't change. The app's `main` belongs to AppKit, so it's a plain `fn main`, not `#[tokio::main]`. Async functions from your library run on a tokio runtime the app keeps. `cargo ferrokit` builds, checks and bundles that binary with its feature on.

This page uses a small time tracker, `tally`, as the example: a library with async `load` and `save`, and a CLI with `start`, `stop` and `report`.

## Where does the GUI go?

In the same package, as a second binary that only builds with the `gui` feature:

```toml
[dependencies]
# …the CLI's own dependencies…
ferrokit = { version = "0.9", optional = true }

[features]
gui = ["dep:ferrokit"]

[[bin]]
name = "tally"
path = "src/main.rs"

[[bin]]
name = "tally-gui"
path = "src/bin/tally-gui.rs"
required-features = ["gui"]

[package.metadata.ferrokit]
bin = "tally-gui"
name = "Tally"
id = "com.example.tally"
```

- `cargo build` and `cargo install tally` build the CLI alone, as before. Nobody who only wants the CLI downloads Ferrokit.
- `bin = "tally-gui"` tells `cargo ferrokit` which binary is the app. `cargo ferrokit run`, `check` and `bundle` then build it with the `gui` feature turned on, and `check` lints and tests with it too. (This needs `cargo-ferrokit` newer than 0.9.0; with 0.9.0, use `cargo run --features gui --bin tally-gui`.)
- In a workspace, a new member package that depends on your library works as well. Run `cargo ferrokit` in its folder.

## What happens to `#[tokio::main]`?

The CLI keeps it. The app can't have it: AppKit needs the main thread for as long as the app runs, and `App::run` doesn't return until the app quits. So the app keeps a tokio runtime of its own and calls the library from it:

- **Before the app starts,** `runtime().block_on(tally::load())` is fine: nothing is on screen yet.
- **While it runs,** never block the main thread. Run the library's async code on another thread with `background(|| runtime().block_on(…))` inside a `spawn_local` task, or with `spawn(work, done)`.

Here is the whole app half, with a stand-in for the library so it compiles on its own:

```rust,no_run
{{#include ../snippets/src/bin/gui_for_cli.rs}}
```

## What if the CLI changes the data while the window is open?

Both read and write the same file, so the window should follow. The example looks at the file's modification time once a second, in a `spawn_local` loop that sleeps on the main run loop, reads the file on another thread, and sets the signal only when the entries changed. That last part matters: `TableModel::set_rows` clears the table's selection, so reloading identical data every second would keep clearing what the user selected.

Save through the library's own `save` from the app too, so both write the same format.

## How do I check it?

`cargo ferrokit check` runs clippy, the tests, a bundle, and then the app with `CHECK=all`, where the app clicks through its own window and prints `[check] name PASS|FAIL` lines. Point those checks at a temporary data file (an environment variable your library already reads, like `TALLY_FILE` here), never the user's own. They only run under `CHECK=all`, so they can stay in the release build.

## Does an AI coding agent get this right?

We tested exactly this on 2026-11-15: a fresh coding agent, given a tokio CLI like this one and asked for "a native Mac GUI for it, with the ferrokit crate", built a working app in about ten minutes with no compile errors and six passing self-checks. It chose the layout above by itself. The rough edges it hit (choosing the binary, live combo box suggestions, a word in an empty date cell) are fixed for the next release, and [`AGENTS.md`](AGENTS.md), which agents read first, now has a section on exactly this case. Run `cargo ferrokit agents` in your project to put that file where your agent will find it.

## See also

- [Async code in a native Mac app](async-in-a-mac-app.md): `spawn_local`, `background`, tokio and reqwest.
- [A native Mac app in Rust, step by step](native-mac-app-in-rust.md): starting from scratch instead.
- [Shipping to the Mac App Store](mac-app-store.md): once the app is ready.
