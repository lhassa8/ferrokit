# Getting started

`ferrokit` needs macOS 15 or later and a stable Rust toolchain. Nothing
else: no Xcode project, no Swift, no build script.

The quickest start is `cargo ferrokit`, which makes a working app with
self-checks, then runs, checks and bundles it:

```bash
cargo install cargo-ferrokit
```

```bash
cargo ferrokit new my-app
```

```bash
cd my-app && cargo ferrokit run
```

While you work on it, `cargo ferrokit preview` runs it the same way and
then rebuilds and restarts it every time you save a file: its windows
open where you left them, the app doesn't take the focus from your
editor, and a build that fails leaves the running app as it was. Values
kept with `stored` survive the restart; other state starts afresh.

```bash
cargo ferrokit preview
```

`--document` makes a document app instead (New, Open…, Save, Recent,
one window per file). [Testing and shipping](https://docs.rs/ferrokit/latest/ferrokit/guide/ch08_testing_and_shipping/)
covers `check`, `bundle` and `notarize`.

By hand, add `ferrokit` to a binary crate:

```toml
[dependencies]
ferrokit = "0.9"
```

## A first app

```rust,no_run
use ferrokit::*;

fn main() {
    let count = Signal::new(0);
    App::new("Counter")
        .window(
            Window::new("Counter").size(320.0, 140.0).view(
                vstack((
                    text(move || format!("Clicked {} times", count.get())),
                    button("Click me", move || count.update(|n| *n += 1)),
                ))
                .padding(20.0),
            ),
        )
        .run();
}
```

What happens here:

- `App::new` names the app. It gets the standard menu bar (the app menu,
  Edit, Window, Help) without asking.
- `Window::new(…).view(…)` builds the window's views **once**. The
  `text` given a closure reads `count`, so when `count` changes, that one
  `NSTextField` gets its new text. Nothing else is rebuilt. Most things a
  view shows work this way: a value, or a closure over signals.
- `button` takes the action to run on click.
- `run` starts AppKit's run loop and doesn't return.

Signals are `Copy` handles: each closure takes its own with `move`, as
above, and nothing needs cloning. A signal is freed with the window, row or
document that made it.

## Main thread only

AppKit is single-threaded, and so is `ferrokit`: views and signals live on
the main thread. Long work goes to a background thread with `spawn`,
which hands the result back to the main thread (see [State](https://docs.rs/ferrokit/latest/ferrokit/guide/ch02_state/)).

## A GUI for an existing project

A Rust project that already has a library or a command-line tool can
add its Mac app as a second binary in the same package. Put it behind a
feature, so the tool's build and `cargo install` stay as they are:

```toml
[dependencies]
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
```

With `bin` set, `cargo ferrokit run`, `check` and `bundle` build
`tally-gui` with the `gui` feature. (In a workspace, a new member package
that depends on the library works as well.)

The app's `main` is a plain `fn main` that ends in `App::run`: AppKit
needs the main thread for as long as the app runs, so it can't be a
`#[tokio::main]` function. A library with `async` functions runs them on
a tokio runtime the app keeps, from a background thread:

```rust,ignore
use std::sync::OnceLock;
use ferrokit::*;

fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Runtime::new().expect("a tokio runtime"))
}

fn main() {
    // Once, before the app starts: blocking here is fine.
    let entries = Signal::new(runtime().block_on(mylib::load()).unwrap_or_default());
    // Later, from a button: the save runs on another thread, and the
    // result comes back on the main thread.
    let save_now = move || {
        let list = entries.get_untracked();
        spawn(move || runtime().block_on(mylib::save(list)), |result| {
            if let Err(e) = result {
                eprintln!("not saved: {e}");
            }
        });
    };
    App::new("Tally")
        .window(Window::new("Tally").view(button("Save", save_now)))
        .run();
}
```

## Opening windows later

`App::window` opens windows at launch. `open(Window)` opens one while the
app runs; closing it (the close button, ⌘W, or `close_window`) tears its
views down and frees it.

```rust,no_run
use ferrokit::*;

fn about() {
    open(Window::new("About").size(300.0, 120.0).view(text("Made with ferrokit").padding(20.0)));
}
# fn main() { about(); }
```

