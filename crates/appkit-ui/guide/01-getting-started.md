# Getting started

`appkit-ui` needs macOS 15 or later and a stable Rust toolchain. Nothing
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

`--document` makes a document app instead (New, Open…, Save, Recent,
one window per file). [Testing and shipping](08-testing-and-shipping.md)
covers `check`, `bundle` and `notarize`.

By hand, add `appkit-ui` to a binary crate:

```toml
[dependencies]
appkit-ui = "0.3"
```

## A first app

```rust,no_run
use appkit_ui::*;

fn main() {
    let count = Signal::new(0);
    App::new("Counter")
        .window(
            Window::new("Counter").size(320.0, 140.0).view(
                vstack((
                    label(move || format!("Clicked {} times", count.get())),
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
- `Window::new(…).view(…)` builds the window's views **once**. `label`
  reads `count`, so when `count` changes, that one `NSTextField` gets its
  new text. Nothing else is rebuilt.
- `button` takes the action to run on click.
- `run` starts AppKit's run loop and doesn't return.

Signals are cheap handles (`Rc` inside). Clone one for each closure that
needs it, as above.

## Main thread only

AppKit is single-threaded, and so is `appkit-ui`: views and signals live on
the main thread. Long work goes to a background thread with `spawn`,
which hands the result back to the main thread (see [State](02-state.md)).

## Opening windows later

`App::window` opens windows at launch. `open(Window)` opens one while the
app runs; closing it (the close button, ⌘W, or `close_window`) tears its
views down and frees it.

```rust,no_run
use appkit_ui::*;

fn about() {
    open(Window::new("About").size(300.0, 120.0).view(text("Made with appkit-ui").padding(20.0)));
}
# fn main() { about(); }
```

