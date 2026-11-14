# Ferrokit vs egui, Iced, Slint, Tauri, NSH, cacao and objc2 on the Mac

Which Rust GUI should you use for a **Mac app**? It depends mostly on one question: does the app have to run anywhere else?

- **Also Windows, Linux, mobile or the web:** use a cross-platform toolkit: egui, Iced, Slint, Tauri or NSH. Ferrokit is Mac only.
- **Mac only, and it should look and behave like a Mac app:** that's what Ferrokit is for. NSH and raw `objc2` are the other ways to get AppKit views from Rust.

Ferrokit is written by the same person as this page, so read it with that in mind. Facts about the other projects come from their own repositories and docs, checked **2026-11-11**, with links. If something here is wrong or out of date, please [open an issue](https://github.com/lhassa8/ferrokit/issues).

## At a glance

| | How it draws on the Mac | Platforms | Maturity (2026-11-11) |
| --- | --- | --- | --- |
| **Ferrokit** | Stock AppKit controls (`NSTableView`, `NSTextView`…) | macOS 15+ | 0.8, alpha; started Oct 2026; one maintainer; no outside users yet |
| **[egui](https://github.com/emilk/egui)** | Its own renderer (immediate mode, wgpu/OpenGL) | Mac, Windows, Linux, web, Android | 0.37 (Oct 2026); ~32k stars; very active |
| **[Iced](https://github.com/iced-rs/iced)** | Its own renderer (wgpu or CPU) | Mac, Windows, Linux, web | 0.14 (Dec 2025); ~32k stars; active; calls itself experimental |
| **[Slint](https://github.com/slint-ui/slint)** | Its own renderer; the `cupertino` style imitates macOS | Desktop, mobile, web, embedded | 1.18 (Sep 2026); ~25k stars; active; commercial company behind it |
| **[Tauri](https://github.com/tauri-apps/tauri)** | The system web view (WKWebView); UI in HTML, CSS, JS | Mac, Windows, Linux, iOS, Android | 2.12 (Oct 2026), 3.0 in alpha; ~113k stars; very active |
| **[NSH](https://github.com/longbridge/nsh)** | AppKit views and Core Animation, styled by the app | Mac, iOS, Android, Windows, Linux, web | 0.5 (Nov 2026); ~2.5k stars; very active (Longbridge) |
| **[cacao](https://github.com/ryanmcgrath/cacao)** | AppKit controls through safe bindings | Mac (iOS in alpha) | 0.4.0-beta2 (Aug 2023); ~2.1k stars; no commits since Dec 2024 |
| **[objc2](https://github.com/madsmtm/objc2)** | Whatever AppKit you call yourself: bindings, no widget layer | Apple platforms | objc2 0.6.5, objc2-app-kit 0.3.2; the bindings Ferrokit itself uses |

## What the Mac gives you, and who gets it

| | VoiceOver | Input methods (Japanese, Chinese…) | System spell check | A table with a million rows |
| --- | --- | --- | --- | --- |
| **Ferrokit** | AppKit's own tree | The system's (`NSTextView`, `NSTextField`) | Yes (the system's) | `NSTableView`, virtualised |
| **egui** | Via AccessKit, on by default; custom widgets describe themselves ([docs](https://github.com/emilk/egui/blob/main/docs/accessibility.md)) | Supported, fixes still landing ([#8681](https://github.com/emilk/egui/pull/8681)) | Not found | `egui_extras::TableBuilder` lays out visible rows only |
| **Iced** | Not in a release yet ([#1849](https://github.com/iced-rs/iced/pull/1849)) | Added in 0.14 | Not found | `table` widget (0.14); virtualisation not found |
| **Slint** | Via AccessKit, `accessible-*` properties | Not confirmed on desktop | Not found | `StandardTableView`, visible rows only ([docs](https://docs.slint.dev/latest/docs/slint/reference/std-widgets/views/standardtableview/)) |
| **Tauri** | What WebKit and your HTML provide | What WebKit provides | What WebKit provides | A JavaScript grid library of your choice |
| **NSH** | The real view hierarchy, with labels, roles and states ([docs](https://nsh.dev/docs/accessibility)) | Handled (inputs are `NSTextField`/`NSTextView`) | Not stated | `DataTable`, cells in view only ([docs](https://nsh.dev/elements/data-table)) |
| **cacao** | Whatever the wrapped controls do | Same | Same | `ListView` with cached rows |
| **objc2** | Whatever you build | Same | Same | `NSTableView`, if you write its data source and delegate |

"Not found" means we found nothing about it in the project's own docs or code; tell us if it exists.

## Shipping

| | Bundle and sign | Notarise | Mac App Store | Updates |
| --- | --- | --- | --- | --- |
| **Ferrokit** | `cargo ferrokit bundle` | `cargo ferrokit notarize` | `bundle --app-store` (sandboxed, universal `.pkg`) | Sparkle, `cargo ferrokit appcast` |
| **Tauri** | Tauri's bundler | Yes, with Apple credentials ([docs](https://v2.tauri.app/distribute/sign/macos/)) | Documented path ([docs](https://v2.tauri.app/distribute/app-store/)) | Tauri's updater plugin |
| **Slint** | Generates an Xcode project | Documented | Documented | Not covered |
| **NSH** | `nsh package` (ad hoc signed `.app`) | By hand, per its docs | Not found for macOS | Not found |
| **egui, Iced, cacao, objc2** | Not in the project (community tools such as `cargo-bundle`) | By hand | By hand | By hand |

## When to pick which

**Pick egui** for tools, debug UIs, editors and anything that also runs on Windows, Linux or the web, when a consistent look of its own is fine. It's the quickest way from nothing to a window, and it's mature.

**Pick Iced** for a typed, Elm-style architecture across desktop platforms, if you can wait for its accessibility work.

**Pick Slint** for a designer-friendly markup language, embedded devices, or one UI on desktop and mobile. Check its licence: royalty-free for desktop, mobile and web apps (not embedded), GPLv3, or commercial.

**Pick Tauri** if you or your team know web development, or you want one UI on desktop and mobile with the largest ecosystem. The UI is HTML and CSS in WebKit; it can look Mac-like with work, but it's a web page.

**Pick NSH** for native views on several platforms from one Rust (or TSX) codebase, with your own design system on top. It's the closest project to Ferrokit in spirit; the difference is that NSH is cross-platform and lets the app own the controls' look, while Ferrokit is Mac only and uses AppKit's stock controls as they are.

**Use objc2 directly** to add one AppKit view or call to an app built with something else, or for complete control. It's what Ferrokit is built on; Ferrokit's [`native(..)`](wrap-any-nsview.md) lets you mix the two.

**cacao** pioneered safe AppKit bindings in Rust, but it hasn't had a release since 2023 or a commit since 2024 and pins an old objc2 beta, so it's hard to recommend for a new app.

**Pick Ferrokit** when the app is Mac only, should behave exactly like a Mac app (tables, outlines, text, menus, VoiceOver, Dark Mode, Liquid Glass) without you rebuilding any of it, and you'd rather write Rust than Swift. Go in knowing what it is today: an alpha with one maintainer and no outside users yet, whose API will still change. The [examples](https://github.com/lhassa8/ferrokit#the-example-apps) show how far it goes, and the [roadmap](https://github.com/lhassa8/ferrokit/blob/main/ROADMAP.md) where it's heading.

## Coming from egui, Iced or Slint

The ideas carry over, with a few differences:

- **Views are built once.** Unlike egui, nothing re-runs every frame; unlike Iced, there's no `view` function rebuilding a tree after each message. A view reads signals, and when a signal changes only the native views that read it update ([State](guide/02-state.md)).
- **State is signals,** `Signal<T>`, which are `Copy`: closures take them with `move`. It's closer to Slint's properties or SolidJS than to Elm's messages.
- **Deep widgets have models.** A table is a `TableModel` of your own row type, created once and fed with `set_rows`; a text editor is a `TextModel`. You don't re-describe a million rows each frame ([Tables, outlines and editors](guide/05-tables-outlines-editors.md)).
- **Menus, shortcuts and the toolbar are the system's:** commands, handled where the focus is ([The app shell](guide/06-app-shell.md)).
- **Styling is AppKit's:** system colours and fonts, materials, SF Symbols. You style less and get the system look ([Styling](guide/09-styling.md)).
- **Everything runs on the main thread;** `spawn` does background work and hands the result back, and `spawn_local` runs async code there, awaiting work on other threads or your own tokio runtime ([Async code in a native Mac app](async-in-a-mac-app.md)).
