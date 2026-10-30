# Testing and shipping

## Self-checks

The example apps test themselves: run with `CHECK=all` and they drive their
own UI, print `[check] <name> PASS|FAIL …` lines and exit non-zero on a
failure. The pattern is simple to copy:

```rust,no_run
use appkit_ui::*;

fn main() {
    let name = Signal::new(String::new());
    App::new("Form")
        .window(Window::new("Form").view(text_field(&name).accessibility_label("Name").padding(20.0)))
        .on_launch(move || {
            if std::env::var("CHECK").as_deref() != Ok("all") {
                return;
            }
            name.set("Ada".into());
            let problems = testing::ax_audit(&windows()[0]);
            let ok = problems.is_empty();
            eprintln!("[check] accessibility {} {problems:?}", if ok { "PASS" } else { "FAIL" });
            std::process::exit(if ok { 0 } else { 1 });
        })
        .run();
}
```

`appkit_ui::testing` has the helpers the suites use:

| Helper | For |
| --- | --- |
| `ax_audit`, `ax_elements`, `ax_tree` | what VoiceOver reads; controls without a name |
| `post_edit`, `send_action`, `shortcut`, `menu_key` | typing, clicks and shortcuts without a mouse |
| `snapshot_windows`, `capture_window`, `captured_pixels` | images of windows, in process or as the window server shows them |
| `footprint_mb`, `peak_footprint_mb` | memory, as Activity Monitor shows it |
| `is_misspelled`, `context_menu_for_word` | the text system |

Unattended runs see AppKit's real behaviour in most places. Two exceptions:
clicks posted to a window that isn't active are ignored, so drive controls
through their own entry points (as the helpers do); and mouse drags between
apps need a person.

In this repository, `scripts/check-all.sh` builds everything, runs clippy,
the unit and doc tests, and every app's suite, and prints a summary.

## Packaging: `cargo ferrokit`

`cargo ferrokit` (install it with `cargo install cargo-ferrokit`) does the
packaging. The app's settings live in its `Cargo.toml`:

```toml
[package.metadata.ferrokit]
name = "My App"            # in the menu bar, Finder and Dock
id = "com.example.myapp"   # the bundle identifier
icon = "icon.png"          # a square PNG (1024 px) or .icns
min-macos = "15.0"
[[package.metadata.ferrokit.documents]]  # files the app opens
name = "Text"
extensions = ["txt", "md"]
```

```bash
cargo ferrokit run
```

```bash
cargo ferrokit check
```

```bash
cargo ferrokit bundle --zip
```

- `run` bundles a debug build and runs it, so it has its name, icon and
  identity, with its output in the terminal.
- `check` runs clippy (warnings are errors) and the tests, bundles, runs
  the self-checks (the app with `CHECK=all`, as above) and verifies the
  signature.
- `bundle` makes a signed `My App.app` with the hardened runtime: ad hoc,
  or with `sign` from `Cargo.toml`. `--universal` builds for Apple silicon
  and Intel (after `rustup target add x86_64-apple-darwin`).

In this repository, `scripts/package-app.sh` does the same for examples.

## Notarisation

To hand the app to other people, sign it with a Developer ID and have Apple
notarise it. Once per machine, save your notary credentials (an
app-specific password from account.apple.com, for the Apple ID of your
developer account):

```bash
xcrun notarytool store-credentials my-profile --apple-id you@example.com --team-id TEAMID
```

Set `sign = "Developer ID Application: Your Name (TEAMID)"` and
`notary-profile = "my-profile"` in `[package.metadata.ferrokit]`. Then one
command builds, signs, notarises, staples and zips (`--dry-run` stops
before the upload):

```bash
cargo ferrokit notarize
```

The zip opens on any Mac running macOS 15 or later with no Gatekeeper
warning.
