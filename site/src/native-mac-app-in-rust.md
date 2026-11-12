# A native Mac app in Rust, step by step

This page goes from an empty folder to a signed, notarised `.app` you can send to someone, with Ferrokit. The app's UI is real AppKit: the same `NSWindow`, `NSTableView` and `NSTextField` a Swift app would use, driven from Rust. No Xcode project, no Swift, no web view.

You need macOS 15 or later and stable Rust.

## 1. Make the app

```bash
cargo install cargo-ferrokit
```

```bash
cargo ferrokit new my-app && cd my-app
```

That's a small to-do app: a text field, a table of tasks with checkboxes, a menu command with a shortcut, and self-checks. It also has an `AGENTS.md`, so Claude Code, Codex or Cursor know how a Ferrokit app is written.

## 2. Run it as a real app

```bash
cargo ferrokit run
```

`run` builds a `.app` bundle, so the app has its name in the menu bar, its icon in the Dock and its own identity, and runs it with its output in the terminal. While you work:

```bash
cargo ferrokit preview
```

rebuilds and restarts it every time you save, with its windows where you left them.

## 3. Write the UI

A Ferrokit app is views over signals. A view is built once; when a signal it reads changes, that one native view updates in place.

```rust,no_run
{{#include ../snippets/src/bin/counter.rs}}
```

The [guide](guide/01-getting-started.md) goes on from here: [state](guide/02-state.md), [layout](guide/03-views-and-layout.md), [controls and forms](guide/04-controls-and-forms.md), [tables and editors](guide/05-tables-outlines-editors.md), and [menus, toolbars, sheets and documents](guide/06-app-shell.md).

## 4. Check it

```bash
cargo ferrokit check
```

Clippy (warnings are errors), your tests, a bundle, then the app's own self-checks: the app runs with `CHECK=all`, drives its own UI (clicks, typing, menu commands) and reports what AppKit did. See [Testing and shipping](guide/08-testing-and-shipping.md).

## 5. Ship it

Settings live in `Cargo.toml`:

```toml
[package.metadata.ferrokit]
name = "My App"
id = "com.example.myapp"
icon = "icon.png"
```

```bash
cargo ferrokit bundle --universal --zip
```

makes a signed `My App.app` with the hardened runtime for Apple silicon and Intel. With a Developer ID certificate, `cargo ferrokit notarize` signs it, sends it to Apple's notary service, staples the ticket and zips it for sending. For updates, `cargo ferrokit appcast` and Sparkle; for the store, [the Mac App Store page](mac-app-store.md).
