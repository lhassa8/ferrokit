# Roadmap

Where Ferrokit is going, roughly in order. It changes as people tell us what they need; the "What I'm building" issues weigh most.

## Now: 0.6

- **Use in real apps.** Rowboat, a SQLite editor built on Ferrokit, is the first app submitted to the Mac App Store. What it runs into becomes Ferrokit work.
- **Gaps people have asked for:**
  - opening the app from a custom URL scheme (`myapp://…`);
  - async/await: run a future and update signals with its result, without writing the threading yourself.
- **Contributor experience:** starter issues, a clearer guide for people coming from egui, Iced or Slint.

## Next

- More of AppKit wrapped as needed (tell us which).
- Fewer places where an app has to reach for `objc2` directly.

## 1.0

1.0 comes once a few independent apps are built on Ferrokit, not on a date. Before it:

- a last API review, deprecated items removed, anything that might change made private;
- a migration guide from 0.x;
- a support policy one person can keep: semver, the current macOS and the one before it at least, and how fast fixes for AppKit changes and security issues come.

## Not planned

- Windows, Linux, iOS. Ferrokit is Mac-only on purpose; maybe later, not now.
- macOS versions before 15.
