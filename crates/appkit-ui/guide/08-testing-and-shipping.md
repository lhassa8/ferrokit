# Testing and shipping

## Self-checks

The example apps test themselves: run with `CHECK=all` and they drive their
own UI, print `[check] <name> PASS|FAIL …` lines and exit non-zero on a
failure. The pattern is simple to copy:

```rust,no_run
use appkit_ui::*;

fn main() {
    let name = Signal::new(String::new());
    let n = name.clone();
    App::new("Form")
        .window(Window::new("Form").view(text_field(&name).accessibility_label("Name").padding(20.0)))
        .on_launch(move || {
            if std::env::var("CHECK").as_deref() != Ok("all") {
                return;
            }
            n.set("Ada".into());
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

## Packaging

`scripts/package-app.sh` builds a release binary and wraps it in a signed
`.app` (Info.plist from your flags and `Cargo.toml`, an icon from a PNG,
the hardened runtime):

```bash
scripts/package-app.sh -p my-app --bin my-app --name "My App" --icon icon.png --zip
```

Add `--universal` for Apple silicon and Intel (after `rustup target add
x86_64-apple-darwin`).

## Notarisation

To hand the app to other people, sign it with a Developer ID and have Apple
notarise it. Once per machine, save your notary credentials (an
app-specific password from account.apple.com, for the Apple ID of your
developer account):

```bash
xcrun notarytool store-credentials my-profile --apple-id you@example.com --team-id TEAMID
```

Then one command builds, signs, notarises, staples and zips:

```bash
scripts/package-app.sh -p my-app --bin my-app --name "My App" --sign "Developer ID Application: Your Name (TEAMID)" --notarize my-profile
```

The zip opens on any Mac running macOS 15 or later with no Gatekeeper
warning.
