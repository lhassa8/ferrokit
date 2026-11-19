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

It's an alpha (0.10, November 2026): the API still changes between minor versions, it's made by one person with an AI coding assistant, and it has no outside users yet. Against that: about 550 self-checks across 18 example apps run the real apps on every change, on macOS 15 and 26, and Rowboat, a SQLite editor built with it, has been submitted to the Mac App Store. Pin the version and read the [changelog](https://github.com/lhassa8/ferrokit/blob/main/CHANGELOG.md) before upgrading.

## How does Ferrokit compare with egui, Iced, Slint, Tauri and GPUI?

They're cross-platform and draw their own controls (Tauri uses a web view); Ferrokit is Mac only and uses AppKit's. Pick one of them if the app must run on Windows or Linux, or is a tool, a game or a debug UI. Pick Ferrokit when it's a Mac app that should behave exactly like one. The [comparison page](compare.md) has the details, with links to each project's docs.

## Why not write the Mac app in Swift?

If you're happy in Swift, do. Ferrokit is for Rust developers who want the whole app in one language: no Swift layer, no FFI boundary between your UI and your logic, one `cargo` build. It's built on [objc2](https://github.com/madsmtm/objc2), the standard Rust bindings to Apple's frameworks.

## Can I add a GUI to my existing Rust CLI or library?

Yes: as a second binary in the same package, behind a feature, so the CLI's build doesn't change. [A native Mac GUI for an existing Rust CLI](gui-for-an-existing-rust-cli.md) shows the `Cargo.toml`, what to do with `#[tokio::main]`, and how the window follows a data file the CLI changes.

## Does Ferrokit work with async code, tokio and reqwest?

Yes. `spawn_local` runs a future on the main thread, where it can set signals between `.await`s; `background` runs blocking work on another thread. For tokio libraries, keep a tokio runtime in the app and await its join handles. See [Async code in a native Mac app](async-in-a-mac-app.md).

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

Yes; it's written to. The crate ships an [`AGENTS.md`](AGENTS.md) with the rules, recipes that compile and known limits, plus [`API.md`](API.md) with every public item; `cargo ferrokit agents` puts the first into your project. Agents can verify their own work with `cargo ferrokit check`, which runs the app's self-checks. In our latest trial, a fresh agent added a working GUI to an existing tokio CLI in about ten minutes with no compile errors. See [For AI coding agents](agents.md) and [`llms.txt`](llms.txt).

## What licence is Ferrokit under?

MIT or Apache-2.0, at your option, like most Rust crates.

## How do I start?

```bash
cargo install cargo-ferrokit && cargo ferrokit new my-app && cd my-app && cargo ferrokit run
```

Then [the guide](guide/01-getting-started.md). If you build something with it, [say so in an issue](https://github.com/lhassa8/ferrokit/issues/new?template=use-case.md).
