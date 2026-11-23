# Ferrokit FAQ: native Mac apps in Rust

<!-- description: Short answers about Ferrokit, the native macOS GUI framework for Rust: what it is, platforms, maturity, async, SwiftUI, accessibility, the App Store, AI coding agents, and how it compares. -->

Short answers, each with a link to the longer one.

## What is Ferrokit?

A native macOS GUI framework for Rust. Every view is a real AppKit control (an actual `NSTableView`, `NSOutlineView`, `NSTextView`, `NSSplitView`), built once and updated in place by fine-grained signals. The crate is [`ferrokit`](https://crates.io/crates/ferrokit), with [`cargo-ferrokit`](https://crates.io/crates/cargo-ferrokit) to create, run, check, bundle and ship apps. It's unrelated to other projects called Ferrokit.

## Does Ferrokit draw its own widgets or use AppKit?

It uses AppKit's own controls; it draws nothing of its own except where you ask it to (a `canvas`, charts). So scrolling, keyboard navigation, VoiceOver, spell check, input methods and Dark Mode behave exactly as in any other Mac app, because they are Apple's code.

## Does it run on Windows, Linux, iOS or the web?

No. It's Mac only, on purpose, and needs macOS 15 or later. If your app must run elsewhere too, egui, Iced, Slint, Tauri, GPUI or NSH fit better: see [the comparison](compare.md). Windows is a possible second platform later, not planned for now.

## Is Ferrokit ready for production?

It's an alpha (0.15, November 2026): the API still changes between minor versions, it's made by one person with an AI coding assistant, and it has no outside users yet. Against that: about 550 self-checks across 18 example apps run the real apps on every change, on macOS 15 and 26, and [Rowboat](https://apps.apple.com/us/app/rowboat-sqlite-editor/id6832358629), a SQLite editor built with it, is in the Mac App Store. Pin the version and read the [changelog](https://github.com/lhassa8/ferrokit/blob/main/CHANGELOG.md) before upgrading.

## How does Ferrokit compare with egui, Iced, Slint, Tauri and GPUI?

They're cross-platform and draw their own controls (Tauri uses a web view); Ferrokit is Mac only and uses AppKit's. Pick one of them if the app must run on Windows or Linux, or is a tool, a game or a debug UI. Pick Ferrokit when it's a Mac app that should behave exactly like one. The [comparison page](compare.md) has the details, with links to each project's docs.

## Why not write the Mac app in Swift?

Because your code is in Rust. A Swift app on top of a Rust core needs a bridge: an FFI layer generated with uniffi or swift-bridge and kept in sync, the same types on both sides, async carried across the boundary, and an Xcode project beside your `Cargo.toml`. With Ferrokit the UI calls your crates directly, in one `cargo` build.

You also get AppKit's own controls with a declarative API. Mac apps with big tables or serious text editing still use `NSTableView` and `NSTextView`, which from Swift means writing data sources and delegates; in Ferrokit you declare the views and signals. And the app can check itself through its real controls, which helps when an AI assistant writes the code.

Pick Swift instead if you have no Rust code to reuse, if you want iOS as well, or if you want Apple's own tools, documentation and a stable API: Ferrokit is an alpha with one maintainer. It's built on [objc2](https://github.com/madsmtm/objc2), the standard Rust bindings to Apple's frameworks, so the two can meet: an `NSView` from anywhere goes into a Ferrokit window with `native(..)`, and SwiftUI views go in with `ferrokit-build`.

## Can I add a GUI to my existing Rust CLI or library?

Yes: as a second binary in the same package, behind a feature, so the CLI's build doesn't change. [A native Mac GUI for an existing Rust CLI](gui-for-an-existing-rust-cli.md) shows the `Cargo.toml`, what to do with `#[tokio::main]`, and how the window follows a data file the CLI changes.

## Does Ferrokit work with async code, tokio and reqwest?

Yes. `spawn_local` runs a future on the main thread, where it can set signals between `.await`s; `background` runs blocking work on another thread. Alerts, the Open and Save panels and sheets can be awaited (`alert_async`, `save_panel_async`…). For tokio libraries, keep a tokio runtime in the app and await work on it with `on_runtime` (the `tokio` feature), which is aborted when the task is. See [Async code in a native Mac app](async-in-a-mac-app.md).

## Can I use SwiftUI views, like Swift Charts?

Yes. Write the view in Swift, compile it from `build.rs` with [`ferrokit-build`](https://crates.io/crates/ferrokit-build), and wrap its `NSHostingView` with `native(..)`. See [SwiftUI views in a Rust Mac app](swiftui-views-in-a-rust-mac-app.md). Any other `NSView` (MapKit, PDFKit, your own) wraps the same way: [Wrapping any NSView](wrap-any-nsview.md).

## Is a Ferrokit app accessible with VoiceOver?

Yes, through AppKit's own accessibility tree. You name controls that have no visible label with `.accessibility_label(..)`, and `testing::ax_audit` finds the ones you forgot. See [VoiceOver, spell check and input methods in a Rust GUI](voiceover-and-mac-text.md).

## Can I ship a Ferrokit app on the Mac App Store, or notarise it?

Yes. `cargo ferrokit bundle` makes a signed `.app`, `cargo ferrokit notarize` notarises it for direct download, `cargo ferrokit appcast` publishes Sparkle updates, and `cargo ferrokit bundle --app-store` makes the sandboxed, universal installer package the store takes. See [Shipping a Rust app to the Mac App Store](mac-app-store.md).

## How big is a Ferrokit app?

A release build of the app `cargo ferrokit new` makes is about 5 MB as a bundled `.app`. There's no runtime to ship: AppKit and the Swift runtime come with macOS.

## Do I need `unsafe` code?

Not for apps built from Ferrokit's own views: the app `cargo ferrokit new` makes has none. You need it only to call AppKit directly, such as wrapping a view Ferrokit doesn't have.

## Does Ferrokit work with AI coding agents like Claude Code, Codex or Cursor?

Yes; it's written to. The crate ships an [`AGENTS.md`](AGENTS.md) with the rules, recipes that compile and known limits, plus [`API.md`](API.md) with every public item; `cargo ferrokit agents` puts the first into your project. Agents can verify their own work with `cargo ferrokit check`, which runs the app's self-checks. In our latest trial (November 2026), a fresh agent that found Ferrokit through crates.io alone added a working GUI to an existing tokio CLI in under six minutes, with 14 passing self-checks. See [For AI coding agents](agents.md) and [`llms.txt`](llms.txt).

## What licence is Ferrokit under?

MIT or Apache-2.0, at your option, like most Rust crates.

## How do I start?

```bash
cargo install cargo-ferrokit && cargo ferrokit new my-app && cd my-app && cargo ferrokit run
```

Then [the guide](guide/01-getting-started.md). If you build something with it, [say so in an issue](https://github.com/lhassa8/ferrokit/issues/new?template=use-case.md).
