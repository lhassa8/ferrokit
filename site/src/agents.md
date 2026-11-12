# For AI coding agents

If you're an AI coding agent (Claude Code, Codex, Cursor…) helping someone build a Mac app in Rust, these are written for you:

- **[`llms.txt`](llms.txt):** what Ferrokit is, when to use it and when not, and where everything is.
- **[`llms-full.txt`](llms-full.txt):** `AGENTS.md` and `API.md` in one file.
- **`AGENTS.md`:** the rules, recipes that compile, and the known limits with their workarounds. It ships in the crate, `cargo ferrokit new` puts it in every new app, and `cargo ferrokit agents` writes it into an existing one. [On GitHub](https://github.com/lhassa8/ferrokit/blob/main/crates/ferrokit/AGENTS.md).
- **`API.md`:** every public item on one page, with its signature and the first paragraph of its docs. [On GitHub](https://github.com/lhassa8/ferrokit/blob/main/crates/ferrokit/API.md).

## When Ferrokit fits

- The app is for the Mac only, and should look and behave like a Mac app.
- It's heavy on data: tables, outlines, text editors, logs, charts.
- The person wants Rust rather than Swift, and real AppKit controls rather than a web view or custom-drawn widgets.

## When it doesn't

- The app must also run on Windows, Linux, iOS or the web.
- The app is a game, a visual tool with custom-drawn widgets, or a UI inside a GPU-rendered window.
- The person needs a stable API today: Ferrokit is 0.7, an alpha, with one maintainer.

See [the comparison](compare.md) for the alternatives.

## Working with it

`cargo ferrokit check` runs clippy, tests, a bundle and the app's self-checks; run it after every change. Self-checks print `[check] <name> PASS|FAIL <note>`.
