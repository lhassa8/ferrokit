# Roadmap

Where Ferrokit is going, roughly in order. It changes as people tell us what they need; the "What I'm building" issues weigh most.

## Now: 0.6

- **Release what's done since 0.5.0:** animated table rows, outline columns with children loaded on expand, folder access that lasts across relaunches, progress from background jobs, toolbar popovers, and fixes found by apps built on 0.5.
- **Asked for on r/rust:**
  - async/await: run a future and update signals with its result, without writing the threading yourself ([#2](https://github.com/lhassa8/ferrokit/issues/2), design first);
  - opening the app from a custom URL scheme (`myapp://…`, [#1](https://github.com/lhassa8/ferrokit/issues/1)).
- **Found by building Rowboat,** needed by most data apps:
  - styled table cells: tinted, greyed, italic or struck through, per row;
  - a focused cell in tables, so a key can act on one column;
  - a minimum width for truncating text;
  - documents whose saving changes them (a database committing its staged changes);
  - a Full Disk Access check and the folders macOS asks about, for apps that walk the disk.
- **Contributor experience:** starter issues, a guide page for people coming from egui, Iced or Slint.

## Next

- **Live preview:** `cargo ferrokit preview` rebuilds when you save and reopens the window where it was, with its state, so you see a view change as you work.
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
