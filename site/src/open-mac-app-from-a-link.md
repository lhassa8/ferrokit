# Open your Mac app from a link (`myapp://`)

A Mac app can have its own kind of link, `myapp://…`, that opens it from a web page, an email, another app or `open` in Terminal. Sign-in callbacks (OAuth redirects) and "open this item" links work this way. In Ferrokit it takes two pieces.

**1. Declare the scheme in `Cargo.toml`:**

```toml
[package.metadata.ferrokit]
url-schemes = ["notes"]
```

`cargo ferrokit bundle` (and `cargo ferrokit run`, which builds a bundle too) writes it into the app's `Info.plist` (`CFBundleURLTypes`). macOS learns which app opens `notes://` links the first time the bundled app runs; after that, a link starts the app if it isn't running. A misspelt scheme, such as `"notes://"` or one with a space, stops the build and says what's wrong.

**2. Handle the links with `App::on_open_url`:**

```rust,no_run
{{#include ../snippets/src/bin/open_url.rs}}
```

`on_open_url` gets every link, whether it launched the app or arrived while the app was running, after the windows are up.

Try it:

```bash
cargo ferrokit run
```

```bash
open "notes://open/42"
```

## Things to know

- **Only a bundled app that has run once gets links.** A bundle that's built but never run isn't known to macOS yet (`open` says no application can open the link), and a bare `cargo run` never gets links. Use `cargo ferrokit run`.
- **Several copies, one winner.** When more than one copy of the app declares the scheme (an old build, a copy in `/Applications`), macOS picks one of them, not necessarily the one you just built.
- **Links arrive percent-encoded,** as macOS gives them (`notes://open/my%20note`). Decode them with a crate such as `url` if you need to.
- **Treat a link as untrusted input.** Any web page or app can send your app one. Check it before acting on it, and ask before doing anything destructive.

The guide has it too: [Links into the app](guide/06-app-shell.md#links-into-the-app-url-schemes), with all of the `[package.metadata.ferrokit]` settings in [`cargo ferrokit`'s README](https://github.com/lhassa8/ferrokit/tree/main/crates/cargo-ferrokit).
