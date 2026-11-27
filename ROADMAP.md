# Roadmap

Where Ferrokit is going, roughly in order. It changes as people tell us what they need; the "What I'm building" issues weigh most.

## Now: 0.15

- **Released in 0.15.1:** a guide chapter for people coming from egui, Iced or Slint, and `cargo ferrokit appcast` naming updates without spaces (GitHub Releases broke their links).
- **0.15.0, the third step to 1.0's API:** windows are `WindowRef`s, Ferrokit's own handle, so the everyday API names no AppKit types; what does (`WindowRef::ns`, `native`, the models' views) is an escape hatch that follows objc2's versions. This is the API proposed for 1.0. Beside it: the support policy ([STABILITY.md](STABILITY.md)) and the migration guide from 0.12 ([MIGRATING.md](MIGRATING.md)).
- **Released in 0.14.1:** `set_recent_documents` (an app's own Open Recent list) and `Outline::transparent_background` (an outline on a card, without a lighter panel in Dark Mode).
- **Released in 0.14.0, the second step to 1.0's API:** what's internal is hidden (the models' plumbing, how views are built, Ferrokit's own probes in `testing::probes`), toolbar items can gain options without breaking apps, and what 0.13 deprecated is gone. One more step (0.15) settles the API for 1.0.
- **Released in 0.13.0, the first step to 1.0's API:** the 0.4 deprecations removed, enums and structs that will grow marked so, builders for open-panel options and cell styles, tooltips and accessibility labels that follow signals, and table and text models that choose their own ids. Two more steps (0.14, 0.15) settle the API for 1.0.
- **Released in 0.11.1:** `cargo ferrokit bundle` takes an Icon Composer `.icon` as the app icon, for macOS 26's dark, tinted and clear looks.
- **Released in 0.11.0:** what building a showcase SQLite editor (Rowboat) needed: image cells from bytes, clickable cell icons, short dates and tighter cell spacing for data grids, ↑ and ↓ in a field that filters a list (`on_arrows`), fast column fitting over a million rows, and a memory reading that works in a sandboxed app.
- **Released in 0.10.1:** from a second agent trial, self-checks that wait for slow work (`testing::wait_for`) and answer alerts (`testing::press_sheet_button`), element values in `testing::ax_elements`, and a table's last column that fills the width.
- **Released in 0.10.0:** a GUI beside an existing Rust CLI or library: `cargo ferrokit` builds an app binary that sits behind a feature (`bin = "…"` in `[package.metadata.ferrokit]`), `AGENTS.md` and the guide say where the app goes and what happens to `#[tokio::main]`, and combo boxes take suggestions that change.
- **Released in 0.9.0:** SwiftUI views in a Ferrokit app (Swift Charts, `Gauge`'s styles, MapKit's SwiftUI `Map`) through `native(..)`, with a new crate, `ferrokit-build`, that compiles the app's Swift from `build.rs`; and Ferrokit's accessibility audit sees inside SwiftUI views.
- **Being designed in [#7](https://github.com/lhassa8/ferrokit/issues/7):** the SwiftUI bridge, so a SwiftUI view needs one line naming it instead of hand-written glue.
- **Released in 0.8.0:** links into the app (`myapp://`, [#1](https://github.com/lhassa8/ferrokit/issues/1)) and async tasks ([#2](https://github.com/lhassa8/ferrokit/issues/2)).
- **Released in 0.12.0, the rest of async ([#2](https://github.com/lhassa8/ferrokit/issues/2)):** awaitable alerts, panels and sheets (`alert_async`, `sheet_async`…), async resources (`Resource::new_async`), and `on_runtime` for work on your tokio runtime (the `tokio` feature).
- **Found by agents building apps in trials:** a toolbar button that shows an on/off state, a collapsed split pane that comes back at its old width, thousands grouping in number columns.
- **Contributor experience:** starter issues, and a guide chapter for people coming from egui, Iced or Slint ([chapter 15](https://lhassa8.github.io/ferrokit/guide/15-coming-from-egui-iced-slint.html)).

## Next

- **Preview that keeps state:** `cargo ferrokit preview` restarts the app today, keeping its windows and `stored` values; keeping the rest of its state across a rebuild is next.
- More of AppKit wrapped as apps need it (tell us which), and fewer places where an app has to reach for `objc2` directly.

What people build decides the order: "What I'm building" issues weigh most.

## 1.0

1.0 comes once a few independent apps are built on Ferrokit, not on a date. What had to come first is done:

- the last API review (0.13 to 0.15): deprecated items removed, anything that might change made private;
- a migration guide from 0.x ([MIGRATING.md](MIGRATING.md));
- a support policy one person can keep ([STABILITY.md](STABILITY.md)): semver, the current macOS and the one before it at least, and how fast fixes for AppKit changes and security issues come;
- updates shipped for real: a Mac App Store update (Rowboat 1.1) and a Sparkle update over HTTPS with a notarised Developer ID app.

Still open: a run of the suites on an Intel Mac ([#6](https://github.com/lhassa8/ferrokit/issues/6)).

## Not planned

- Windows, Linux, iOS. Ferrokit is Mac-only on purpose; maybe later, not now.
- macOS versions before 15.
