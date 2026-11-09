# Contributing to Ferrokit

Thanks for looking. Ferrokit is young and mostly one person's work so far, so every issue, question and pull request helps, small ones included.

## Ways to help

- **Tell us what you're building**, or tried to build and couldn't. The "What I'm building" issue template is for that, and it's the most useful thing you can send.
- **Report a bug** with the bug template: what you did, what happened, your macOS version.
- **Ask in [Discussions](https://github.com/lhassa8/ferrokit/discussions)** when you're not sure something is a bug, or want to know how to do something.
- **Pick up an issue** labelled [`good first issue`](https://github.com/lhassa8/ferrokit/labels/good%20first%20issue) or [`help wanted`](https://github.com/lhassa8/ferrokit/labels/help%20wanted). Say in the issue that you're on it, so nobody doubles up.
- **Improve the docs or the guide.** If something confused you, it will confuse the next person.

## Building and checking

You need a Mac with macOS 15 or later and the latest stable Rust.

```sh
cargo build --workspace --examples
cargo run -p ferrokit --example habits      # try an app
scripts/check-all.sh                         # everything, as CI runs it
```

`scripts/check-all.sh` runs the build, clippy (warnings are errors), unit and doc tests, the docs, and each example app's self-checks (`CHECK=all`). The self-checks open real windows and drive them, so leave the Mac alone while they run; the whole run takes a while. Some checks need Accessibility permission for your terminal and print `SKIP` without it; that's fine.

For a quicker loop while you work on one area, run the example that covers it, for instance `CHECK=all cargo run -p ferrokit --example ui_checklist`.

## What a pull request needs

- **A check for new behaviour.** Most changes get one in `crates/ferrokit/examples/ui_checklist.rs`; look at the checks near your change for the pattern. A bug fix gets a check that failed before it.
- **Docs in the same pull request.** Public items have doc comments (both crates use `#![warn(missing_docs)]`). If you change public API, run `scripts/api-md.sh` to regenerate `crates/ferrokit/API.md`, and update the guide chapter and the CHANGELOG's *Unreleased* section.
- **Clean checks:** `cargo clippy --release --workspace --examples -- -D warnings`, `cargo test --workspace`, and `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps`.
- **No new `unsafe` in the public API.** If an app would need `unsafe` to do something ordinary, that's a missing Ferrokit feature, so add it.

CI runs on macOS 15 and the current macOS for every pull request. The self-check suites are allowed to fail there while hosted runners are unreliable for GUI timing, so read their summary, not only the job's colour.

## How Ferrokit is made

Ferrokit is developed with a lot of help from an AI coding assistant (Claude), reviewed and directed by the maintainer. What keeps it honest is the same for everyone's code: every behaviour has a self-check that runs the real app, CI runs on two macOS versions, and the docs compile as tests.

AI-assisted pull requests are welcome on the same terms. Say so in the pull request, and make sure you understand and have run what you're sending.

## Design rules worth knowing

- Views are real AppKit controls, created once and updated in place by signals. Don't recreate native views on every update; AppKit keeps records of every view ever added to a window.
- Ferrokit lays views out itself; no Auto Layout.
- Bindings go through `objc2` and `objc2-app-kit`. Enable only the features you use.
- Table rows use model indices; text uses UTF-8 byte offsets.
- macOS only, 15 and later. Other platforms are out of scope for now.

## Conduct

Be kind and assume good faith. This project follows the [Rust Code of Conduct](https://www.rust-lang.org/policies/code-of-conduct).

## License

By contributing, you agree that your contributions are licensed under MIT OR Apache-2.0, like the rest of Ferrokit.
