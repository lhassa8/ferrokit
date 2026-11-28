# Built with Ferrokit: Rowboat, a SQLite editor in the Mac App Store

<!-- description: Rowboat, a free SQLite editor in the Mac App Store, is written in Rust with Ferrokit: about 17,000 lines, real AppKit tables and text views, a million rows in a fifth of a second, sandboxed and packaged by cargo ferrokit. -->

[Rowboat](https://apps.apple.com/us/app/rowboat-sqlite-editor/id6832358629) is a free SQLite editor for the Mac, written entirely in Rust with Ferrokit. It's the answer to "has anyone shipped a real app with this?": it's in the Mac App Store, in 174 countries, in English and German, for macOS 15 or later.

![Rowboat's main window: a million orders in a table, with the tables, views, indexes and triggers in the sidebar](images/rowboat-main.jpg)

## In numbers

| | |
| --- | --- |
| Code | about 17,000 lines of Rust, plus [`rusqlite`](https://crates.io/crates/rusqlite) with SQLite bundled |
| Started | 6 November 2026, from Ferrokit's `sql_browser` example |
| 1.0 in the store | 18 November 2026 (submitted 8 November) |
| 1.1, the first update | 25 November 2026 |
| Store package | 8.7 MB, universal (Apple silicon and Intel), sandboxed |
| Self-checks | about 118, run through the real app's controls (`CHECK=all`) |
| A million rows | on screen in about a fifth of a second, and they scroll smoothly |

## What it's made of

Apart from the schema diagram, every part of the window is an AppKit control that Ferrokit creates once and updates in place:

- **The data:** `table(..)`, an `NSTableView` that asks only for the rows on screen, which is why a million rows cost nothing to show. Edits are staged in Rust and written when you commit, with undo through `undo(..)`.
- **The SQL editor:** `text_editor(..)`, an `NSTextView`, with highlighting, completion and a formatter written in Rust on top. Input methods, spell check and VoiceOver are the system's.
- **The sidebar:** `outline(..)` in a `sidebar(..)`: tables, views, indexes, triggers and saved snippets.
- **The schema diagram:** a `canvas(..)`, the one place Rowboat draws for itself.
- **Database Health:** Ferrokit's charts (a donut of space per table and index) above a table of every object.
- **The rest of the Mac:** a toolbar, menus, sheets, popovers, an inspector, the Open and Save panels, the pasteboard, Quick Look for images and PDFs in cells, and drag to the Finder.

![The schema diagram, drawn from the database's foreign keys](images/rowboat-diagram.jpg)

## How it got into the store

`cargo ferrokit bundle --app-store` built the sandboxed, universal installer package the store takes; [Shipping a Rust app to the Mac App Store](mac-app-store.md) walks through the same steps. Rowboat has no network access and its privacy label is "Data Not Collected": a sandboxed Rust app needed nothing special to get there.

Updates go through the store. For apps you ship yourself, `cargo ferrokit appcast` publishes Sparkle updates instead (see [Testing and shipping](guide/08-testing-and-shipping.md)).

![Database Health: space per table and index, and the maintenance buttons](images/rowboat-health.jpg)

## What it taught Ferrokit

Rowboat is the framework's daily-use test, and building it changed Ferrokit. Tables gained cell focus (a focused column, as in a spreadsheet), styled cells, animated rows and links dropped from a browser. Text and search fields gained `on_arrows`, for lists filtered as you type, the way Open Quickly works. And it found bugs that every app would have hit: a `canvas` that was never drawn at its minimum size, views showing through a hidden parent, and empty editable cells with no name for VoiceOver. The [changelog](https://github.com/lhassa8/ferrokit/blob/main/CHANGELOG.md) says which changes came from Rowboat.

## Try it, or build your own

- Get Rowboat from the [Mac App Store](https://apps.apple.com/us/app/rowboat-sqlite-editor/id6832358629) (free, no account).
- Its starting point is in Ferrokit's repository: `cargo run --release -p ferrokit --example sql_browser`.
- Start your own: `cargo install cargo-ferrokit && cargo ferrokit new my-app`, then [the guide](guide/01-getting-started.md).

If you've built something with Ferrokit, [tell us in an issue](https://github.com/lhassa8/ferrokit/issues/new?template=use-case.md) and it can be listed here too.
