# Roadmap

Where Ferrokit is going, roughly in order. It changes as people tell us what they need; the "What I'm building" issues weigh most.

## Now: 0.8

- **Released in 0.8.0,** both asked for on r/rust:
  - links into the app: `myapp://` URL schemes declared in `Cargo.toml` and handled by `App::on_open_url` ([#1](https://github.com/lhassa8/ferrokit/issues/1));
  - async tasks, the first layer of async/await: `spawn_local`, `background`, `sleep`, and tasks that stop with their window ([#2](https://github.com/lhassa8/ferrokit/issues/2)).
- **Released in 0.7.0:** `cargo ferrokit preview` (rebuilds and restarts the app on every save, windows where they were), the fixes from a review of the whole framework, Edit ▸ Find in every app, and what AI coding agents need to succeed with Ferrokit: `AGENTS.md` in the crate, `cargo ferrokit agents`, and helpers for self-checks.
- **Being designed in [#2](https://github.com/lhassa8/ferrokit/issues/2):** the rest of async: awaitable panels and alerts, async resources, and an optional helper for tokio. Your answers to the questions there decide them.
- **Found by agents building apps in trials:** a toolbar button that shows an on/off state, a collapsed split pane that comes back at its old width, thousands grouping in number columns.
- **Contributor experience:** starter issues, a guide page for people coming from egui, Iced or Slint.

## Next

- **Preview that keeps state:** `cargo ferrokit preview` restarts the app today, keeping its windows and `stored` values; keeping the rest of its state across a rebuild is next.
- **SwiftUI inside Ferrokit:** wrap a SwiftUI view the way `native(..)` wraps an `NSView`, for the parts of Apple's frameworks that are SwiftUI-only. A design issue first.
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
