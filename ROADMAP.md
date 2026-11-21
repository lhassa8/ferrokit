# Roadmap

Where Ferrokit is going, roughly in order. It changes as people tell us what they need; the "What I'm building" issues weigh most.

## Now: 0.10

- **Released in 0.10.1:** from a second agent trial, self-checks that wait for slow work (`testing::wait_for`) and answer alerts (`testing::press_sheet_button`), element values in `testing::ax_elements`, and a table's last column that fills the width.
- **Released in 0.10.0:** a GUI beside an existing Rust CLI or library: `cargo ferrokit` builds an app binary that sits behind a feature (`bin = "…"` in `[package.metadata.ferrokit]`), `AGENTS.md` and the guide say where the app goes and what happens to `#[tokio::main]`, and combo boxes take suggestions that change.
- **Released in 0.9.0:** SwiftUI views in a Ferrokit app (Swift Charts, `Gauge`'s styles, MapKit's SwiftUI `Map`) through `native(..)`, with a new crate, `ferrokit-build`, that compiles the app's Swift from `build.rs`; and Ferrokit's accessibility audit sees inside SwiftUI views.
- **Being designed in [#7](https://github.com/lhassa8/ferrokit/issues/7):** the SwiftUI bridge, so a SwiftUI view needs one line naming it instead of hand-written glue.
- **Released in 0.8.0:** links into the app (`myapp://`, [#1](https://github.com/lhassa8/ferrokit/issues/1)) and async tasks ([#2](https://github.com/lhassa8/ferrokit/issues/2)).
- **Being designed in [#2](https://github.com/lhassa8/ferrokit/issues/2):** the rest of async: awaitable panels and alerts, async resources, and an optional helper for tokio. Your answers to the questions there decide them.
- **Found by agents building apps in trials:** a toolbar button that shows an on/off state, a collapsed split pane that comes back at its old width, thousands grouping in number columns.
- **Contributor experience:** starter issues, a guide page for people coming from egui, Iced or Slint.

## Next

- **Preview that keeps state:** `cargo ferrokit preview` restarts the app today, keeping its windows and `stored` values; keeping the rest of its state across a rebuild is next.
- More of AppKit wrapped as apps need it (tell us which), and fewer places where an app has to reach for `objc2` directly.

What people build decides the order: "What I'm building" issues weigh most.

## 1.0

1.0 comes once a few independent apps are built on Ferrokit, not on a date. Before it:

- a last API review, deprecated items removed, anything that might change made private;
- a migration guide from 0.x;
- a support policy one person can keep: semver, the current macOS and the one before it at least, and how fast fixes for AppKit changes and security issues come.

## Not planned

- Windows, Linux, iOS. Ferrokit is Mac-only on purpose; maybe later, not now.
- macOS versions before 15.
