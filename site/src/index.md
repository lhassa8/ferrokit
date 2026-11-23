# Ferrokit: native Mac apps in Rust

**Ferrokit is a native macOS GUI framework for Rust.** It puts a real Mac app on top of your Rust code: every view is a real AppKit control (an actual `NSTableView`, `NSOutlineView`, `NSTextView`, `NSSplitView`), built once and updated in place by fine-grained signals.

- **Your Rust code, a real Mac app on top.** The UI calls your crates directly, in one language and one `cargo` build: no Swift layer, no FFI bridge, no types written twice, no Xcode project.
- **The real controls, without the ceremony.** Not a web view, and not a GPU renderer imitating AppKit, so scrolling, keyboard navigation, VoiceOver, spell check, input methods and Dark Mode are Apple's own. You declare views and signals; Ferrokit writes the data sources and delegates.
- **Apps that check themselves.** An app can run its own checks through its real controls, unattended, and every project ships an `AGENTS.md`, so you or your AI assistant can build a change and know it works.

**Built with it:** [Rowboat](https://apps.apple.com/us/app/rowboat-sqlite-editor/id6832358629), a SQLite editor in the Mac App Store, about 17,000 lines of Rust.

It's Mac only, on purpose, and needs macOS 15 or later. If you need Windows or Linux too, read [the comparison](compare.md) first: other toolkits fit that better.

![The SQL browser example: a schema sidebar, a highlighted query, its results table, a bar chart and an inspector](images/sql-browser.png)

## Try it in one command

```bash
git clone https://github.com/lhassa8/ferrokit && cd ferrokit && cargo run --release -p ferrokit --example sql_browser
```

That's a SQLite browser over a generated sample of a million orders: click `orders`, fling the table, sort by `total`, type a query (⌘R runs it), switch to Dark Mode.

Or start your own app:

```bash
cargo install cargo-ferrokit && cargo ferrokit new my-app && cd my-app && cargo ferrokit run
```

## What the code looks like

Views are built once. When a signal changes, only the views that read it update, in place: no virtual DOM, no diffing.

```rust,no_run
{{#include ../snippets/src/bin/counter.rs}}
```

## What you get without writing it

Because each view is the real AppKit control:

- **VoiceOver** reads AppKit's own accessibility tree; `testing::ax_audit` finds controls you forgot to name.
- **Text** is Apple's text system: spell check, dictation, Japanese, Chinese and Korean input, the emoji picker, Find (⌘F), undo per window.
- **Tables and outlines** are `NSTableView` and `NSOutlineView`: a million rows, sorting, arrow keys, ⌘C as tab-separated text, drag to the Finder.
- **The look** follows the system: Dark Mode, accent colours, sidebar materials, Liquid Glass on macOS 26.
- **The system's services:** the Share menu, Quick Look, printing, notifications, menu-bar extras.

## From idea to a shipped app

`cargo ferrokit` makes, runs, checks, bundles, notarises and packages for the Mac App Store, with Sparkle updates and crash reports for apps you ship yourself. See [shipping to the Mac App Store](mac-app-store.md).

## Where things stand

Ferrokit is **an alpha (0.12)**: the API still changes between minor versions, and it's young (started October 2026), made by one person with an AI coding assistant, and has no outside users yet. About 550 self-checks across 18 example apps run the real apps on macOS 15, 26 and the newest beta. [Rowboat](https://apps.apple.com/us/app/rowboat-sqlite-editor/id6832358629), a SQLite editor built with it, is in the Mac App Store.

If you build something with it, even a weekend toy, [say so in an issue](https://github.com/lhassa8/ferrokit/issues/new?template=use-case.md): what people build decides what comes next.

## Links

- [The guide](guide/01-getting-started.md), 14 chapters whose examples compile as tests
- [API reference on docs.rs](https://docs.rs/ferrokit)
- [Source on GitHub](https://github.com/lhassa8/ferrokit) · [crates.io](https://crates.io/crates/ferrokit) · MIT OR Apache-2.0
- [For AI coding agents](agents.md): `AGENTS.md`, `API.md` and [`llms.txt`](llms.txt)
