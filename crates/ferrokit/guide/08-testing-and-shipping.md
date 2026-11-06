# Testing and shipping

## Self-checks

The example apps test themselves: run with `CHECK=all` and they drive their
own UI, print `[check] <name> PASS|FAIL …` lines and exit non-zero on a
failure. The pattern is simple to copy:

```rust,no_run
use ferrokit::*;

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

`ferrokit::testing` has the helpers the suites use:

| Helper | For |
| --- | --- |
| `ax_audit`, `ax_elements`, `ax_tree` | what VoiceOver reads; controls without a name |
| `post_edit`, `send_action`, `shortcut`, `menu_key` | typing, clicks and shortcuts without a mouse |
| `snapshot_windows`, `capture_window`, `captured_pixels` | images of windows, in process or as the window server shows them |
| `footprint_mb`, `peak_footprint_mb` | memory, as Activity Monitor shows it |
| `is_misspelled`, `context_menu_for_word` | the text system |

`cargo ferrokit check` runs the app with `CHECK=all` and counts the
`[check]` lines (a name may have several words); it shows only those lines,
so run the bundled app with `CHECK=all` yourself to see everything it
prints. Things that catch people
out, usually once:

- **One turn to act, the next to look.** A `spawn` or `later` callback runs
  inside a [`batch`](crate::batch), so effects and table updates happen
  after it returns: change something in one step, check it in the next.
- **Undo steps.** AppKit closes an event's undo group only when the app
  handles an event, so in an unattended run the groups stay open across
  turns and one ⌘Z can undo several changes. Make each step its own group:
  `undo.setGroupsByEvent(false)` on the window's `undoManager()`, then
  `beginUndoGrouping()` and `endUndoGrouping()` around each step (the
  `AGENTS.md` that `cargo ferrokit new` writes has the helper).
- **Selecting from code is silent.** `TableModel::select` doesn't call
  `on_select`, and `set_rows` clears the selection.
- **The real settings.** Checks run against the app's own user defaults, so
  [`stored`](crate::stored) values from the last run are still there:
  reset the ones a check depends on at its start.
- **Menu items:** `testing::menu_key` presses a menu's shortcut, including
  your own [`menu_command`](crate::menu_command)s; `testing::shortcut`
  does the same for a ⌘ or ⇧⌘ shortcut without the app being active: your
  own items run if they're enabled, and standard ones (Undo, Copy) go along
  the window's responder chain.
- **AppKit types in checks:** looking at a native view directly needs
  `objc2`, `objc2-foundation` and `objc2-app-kit` (0.6, 0.3 and 0.3) in
  your `Cargo.toml`, with the AppKit features you use.
- **A panic doesn't end the app.** It's caught and reported (see below), so
  `check` stops 15 s after one if no more checks arrive.

Unattended runs see AppKit's real behaviour in most places. Two exceptions:
clicks posted to a window that isn't active are ignored, so drive controls
through their own entry points (as the helpers do); and mouse drags between
apps need a person.

### Real input

`testing::input` sends clicks and keystrokes along the user's path:
- **Event-queue input** works whenever the app is active and the window
  key (`input::ready(&window)`). `input::click(&view)`, `click_at`,
  `double_click`, `type_text(&window, "Ada")` and
  `key(&window, Key::Down, &[Mod::Command])` post events to the app's own
  queue, so AppKit dispatches them as it does the window server's:
  hit-testing, the first responder, the field editor, the menus' key
  equivalents.
- **Window-server input** (`server_click`, `server_type`) goes through the
  window server like a real keyboard and mouse, without moving the
  pointer. It needs the process to be trusted for Accessibility;
  `input::server_available()` says whether it is, and checks report
  `SKIP` when it isn't.

```rust,no_run
use ferrokit::testing::input::{self, Key, Mod};
use ferrokit::*;

fn check(window: &NSWindow, field: &objc2_app_kit::NSView) {
    if input::ready(window) {
        input::click(field);
        input::type_text(window, "Ada Lovelace");
        input::key(window, Key::Char('s'), &[Mod::Command]);
        // Look at the effect a moment later: events arrive on the next turns.
    }
}
# let _ = check;
```

A posted Return doesn't press a window's default button (not even in a
plain AppKit app), so click it instead; window-server input does press it.

The `input` example's suite and the real-input checks in `habits` and
`sql_browser` use it. They found that a shortcut typed straight after text
ran on the old text; menu items, buttons and commands now deliver an
editor's pending changes first.

### Soak tests

`testing::soak` runs an app's own list of user actions (clicks, typing,
page switches, sheets, resizing) at random for a long time, and watches:
- a watchdog thread reports the main thread not answering for over two
  seconds;
- every 30 seconds it samples the footprint and the live views, signals
  and windows;
- at the end it prints the footprint's growth over the second half of
  the run.

```rust,no_run
use ferrokit::testing::input::{self, Key};
use ferrokit::testing::soak::{self, Rng, Settings, action};
use ferrokit::*;

fn start(window: Retained<NSWindow>) {
    let w = window.clone();
    soak::run(Settings::from_env(), vec![
        action("arrow", 3, move |r: &mut Rng| input::key(&w, *r.pick(&[Key::Up, Key::Down]), &[])),
        action("resize", 1, move |r: &mut Rng| {
            let f = window.frame();
            let size = objc2_foundation::NSSize::new(600.0 + r.below(400) as f64, 400.0 + r.below(300) as f64);
            window.setFrame_display(objc2_foundation::NSRect::new(f.origin, size), true);
        }),
    ]);
}
# let _ = start;
```

`SOAK=<minutes>` sets the length, `SOAK_SEED` the seed (printed at the
start, so a run can be repeated) and `SOAK_MS` the time between actions.
The app takes the front at the start and pauses while another app is in
front. The `habits` and `sql_browser` examples have soaks (in Ferrokit's
repository, `scripts/soak.sh` runs one and summarises the log and crash
reports).

## When something fails

An Objective-C exception (AppKit refusing a call: an index out of range,
a row that isn't loaded yet) or a Rust panic in code AppKit calls into
your app, such as a control's action, a menu item, a timer, a notification
or a sheet's completion, is caught where it came in. It's printed to stderr
with where it happened, and the app goes on. Without that, AppKit swallows
the exception, and the rest of that work silently never runs.

`App::on_error(|failure| ..)` hears about each one (log it, report it, tell
the user), and `testing::failures()` lists them for self-checks: a check
can assert that a run caused none.

```rust,no_run
use ferrokit::*;

let app = App::new("Notes").on_error(|failure| eprintln!("to the crash log: {failure}"));
# let _ = app;
```

## Crash reports

`on_error` hears about failures the app survives. A panic Ferrokit doesn't
catch (outside a control's action, menu item, timer and the like, or on a
background thread) ends the app. `App::crash_reports(|report| ..)` writes
a report for each one before the app ends: the app and its version,
macOS, the thread, the panic's message and place, and the backtrace, in
`~/Library/Logs/<app>/Crashes` (inside the container when sandboxed). At
the next launch, once the windows are up, the handler gets each report not
yet marked sent. Panics caught at the places AppKit calls in stay
`on_error` failures and write no report.

What to do with a report is the app's choice. Two common ones:

- **Ask the user.** Show an alert saying the app quit unexpectedly, and
  offer to show the report in the Finder (for attaching to an email or an
  issue). Mark it sent either way, so it isn't asked about again.
- **Send it to a service.** Post `report.text` to your server or a crash
  service from a background thread (`spawn`), and mark it sent when the
  upload succeeds. Sentry's Rust SDK (`sentry` crate) can also capture
  panics itself; if you use it, its own hook replaces this one.

```rust,no_run
use ferrokit::*;

let app = App::new("Notes").crash_reports(|report| {
    let Some(window) = windows().into_iter().next() else { return };
    let path = report.path.clone();
    alert(
        &window,
        Alert::new("Notes quit unexpectedly last time.", "A report was saved. You can attach it to an email to the developer.")
            .button("Show Report")
            .button("Not Now"),
        move |choice| {
            if choice == 0 {
                reveal_in_finder(&[&path]);
            }
        },
    );
    let _ = report.mark_sent();
});
# let _ = app;
```

A crash that isn't a panic, such as a bad memory access in unsafe code or
a C library, ends the app at once. macOS keeps those in
`~/Library/Logs/DiagnosticReports` (the user sees them in Console), and
offers to send them to Apple, not to you. Uploading a build's debug
symbols isn't needed for Rust panics: the backtrace names the functions,
as long as release builds keep their symbols (the default; don't set
`strip = true` if you want readable reports).

For checks, `FERROKIT_CRASH_DIR` points the reports at a folder of the
check's own, `testing::pending_crash_reports(dir)` reads them, and
`testing::catching(|| ..)` runs code as Ferrokit runs an action, so a
panic in it is caught. The `ui_checklist` example's `crash-report` check
crashes a child run of itself and reads the report back at its next
launch.

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

## The App Sandbox

The Mac App Store needs the App Sandbox, and any app can opt in for
safety. `sandbox = true` in `[package.metadata.ferrokit]` makes
`cargo ferrokit bundle` sign with its entitlements: files the user
chooses (open and save panels, dropped files) and bookmarks to reach them
again; `network = true` adds outgoing connections. Document apps keep
working sandboxed: Ferrokit keeps a security-scoped bookmark for each
document, so Open Recent and reopening at launch reach the files after a
relaunch.

```toml
[package.metadata.ferrokit]
sandbox = true
network = true
```

`check-all` runs the `notes` example sandboxed. A file outside the app's
container can only be granted by the user choosing it, so try Open Recent
on one by hand once.

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
