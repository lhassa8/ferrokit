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

What else the sandbox must allow goes in a table of entitlements, under
Apple's names: an app that prints (`print(..)`) needs
`com.apple.security.print`. More Info.plist keys (a usage description,
the App Store's export compliance answer) go in a `plist` table. Both
take `true`/`false`, strings and lists of strings.

```toml
[package.metadata.ferrokit.entitlements]
"com.apple.security.print" = true

[package.metadata.ferrokit.plist]
ITSAppUsesNonExemptEncryption = false
```

A document that needs files beside it (a SQLite database's `-journal`,
`-wal` and `-shm`) gets only the document itself from the user's choice.
Declare the others as related files, in the document type
(`DocumentType::related`) and in `Cargo.toml`, and Ferrokit claims them
before reading the document:

```toml
[[package.metadata.ferrokit.documents]]
name = "SQLite Database"
extensions = ["sqlite", "db"]
related = ["-journal", "-wal", "-shm"]
```

Files the app opens but never saves as themselves (a CSV it shows as a
table, saved with Save As… as a database) are a viewer's: say so with
`role = "viewer"` in their document type.

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

## Updates

Apps outside the Mac App Store usually update through
[Sparkle](https://sparkle-project.org), and Ferrokit uses Sparkle 2. When
the app is bundled with it, the app menu gets **Check for Updates…**, and
Sparkle does the rest. At the second launch it asks the user whether to
check automatically. When there's a new version it shows the release notes,
downloads the update, checks its signature, installs it and relaunches.
The app's code doesn't change.

Once:

1. Download Sparkle 2 (`Sparkle-2.x.tar.xz` from
   github.com/sparkle-project/Sparkle/releases) and unpack it, for example
   in `~/Developer/Sparkle-2.10.1`. Its licence is MIT-style: include its
   notice with your app's credits.
2. Make the key that signs your updates. Sparkle's `bin/generate_keys`
   keeps the private key in your login keychain and prints the public
   key. Back the private key up (`generate_keys -x <file>`): without it you
   can't publish updates your users will accept.
3. Tell `cargo ferrokit` where updates come from:

```toml
[package.metadata.ferrokit.updates]
feed = "https://example.com/myapp/appcast.xml"
public-key = "…"                           # what generate_keys printed
sparkle = "~/Developer/Sparkle-2.10.1"      # or set FERROKIT_SPARKLE
```

`cargo ferrokit bundle` (and `run`, `check`, `notarize`) then embeds
`Sparkle.framework`, signs its parts with the app's identity, and puts the
feed and the public key in `Info.plist`. A sandboxed app gets the settings
and entitlements that Sparkle's installer (and, without `network = true`,
its downloader) needs. `bundle --app-store` leaves Sparkle out: the App
Store does the updating there.

Each release:

1. Raise `version` in `Cargo.toml`.
2. `cargo ferrokit notarize` (a Developer ID signature and Apple's ticket).
3. `cargo ferrokit appcast`. It zips the app into `updates/` as
   `My App 1.1.0.zip` and writes `updates/appcast.xml` with Sparkle's
   `generate_appcast`, signing the update with your key.
4. Upload the new files to the feed's folder. Any HTTPS host works: your
   site, S3, GitHub Pages.

Keep `updates/` between releases. `generate_appcast` keeps the feed's
history and makes small delta updates from the earlier versions.

Nothing in the app has to call Sparkle. The `updates` module is for an
app that wants more: `updates::check()` runs a check from a button,
`set_automatic_checks` and `set_automatic_downloads` back a Settings
checkbox, and `updates::on_event` reports what the updater does:

```rust,no_run
use ferrokit::*;

updates::on_event(|event| match event {
    // Downloaded and checked; it installs when the app quits.
    UpdateEvent::ReadyOnQuit { version } => set_dock_badge(Some(version.as_str())),
    UpdateEvent::Failed { message } => eprintln!("update failed: {message}"),
    _ => {}
});
```

Run unbundled (`cargo run`), the app has no Sparkle, and these functions
do nothing (`updates::available()` is false). A bundle that's set up for
updates but can't start Sparkle prints why and reports it to `on_event`.
Ad hoc builds turn off library validation so they can load Sparkle; with
a Developer ID, the app and Sparkle share your team and need no
exception.

`check-ferrokit.sh` (part of `check-all`) tests the whole path when
Sparkle is unpacked in `target/sparkle-<version>` or `FERROKIT_SPARKLE`
names it. It installs version 1.0.0, serves 1.1.0 from a feed on
`127.0.0.1`, shows that an update signed with the wrong key is refused,
and that the right one installs when the app quits. It runs both plain
and sandboxed. A feed on `127.0.0.1` is only for testing like this: a
real feed must be HTTPS.

## The Mac App Store

The Mac App Store takes an installer package of a sandboxed app, signed
with your App Store identities. You need, from your Apple Developer
account:

- an **Apple Distribution** certificate (signs the app) and a **Mac
  Installer Distribution** certificate (signs the package; Keychain Access
  shows it as "3rd Party Mac Developer Installer"), both in your keychain;
- a **Mac App Store provisioning profile** for the app's bundle id
  (developer.apple.com ▸ Profiles), saved next to the app;
- the app in **App Store Connect**, with the same bundle id.

```toml
[package.metadata.ferrokit]
sandbox = true
category = "public.app-category.productivity"
app-store-sign = "Apple Distribution: Your Name (TEAMID)"
installer-sign = "3rd Party Mac Developer Installer: Your Name (TEAMID)"
provisioning-profile = "MyApp.provisionprofile"
```

```bash
cargo ferrokit bundle --app-store --build 42
```

It builds the app for Apple silicon and Intel, embeds the profile, signs
the app with its sandbox entitlements and the profile's app and team ids
(a profile for another app, or for iOS, is refused with the reason),
and writes `My App.pkg`. `--build` is the build number (`CFBundleVersion`;
each upload needs a higher one), and the version is the package's.
`--dry-run` does all of it without your identities: ad hoc signing and an
unsigned package, then the signing commands it would run.

Upload the package with Apple's Transporter app (from the Mac App Store).
Before submitting, run the sandboxed build yourself (`cargo ferrokit run`
with `sandbox = true`): App Review checks that the app works in the
sandbox, and files outside the container are reachable only through open
and save panels, drops and the bookmarks Ferrokit keeps.
