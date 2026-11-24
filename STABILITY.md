# Stability and support

What Ferrokit promises, and what it doesn't. Ferrokit is made by one person, so this is a policy one person can keep: the times below are what to expect, not a contract.

## Versions

Ferrokit follows [semantic versioning](https://semver.org/). Its crates (`ferrokit`, `ferrokit-core`, `cargo-ferrokit`, `ferrokit-build`) share one version and are released together; `ferrokit` pins `ferrokit-core` exactly, so use matching versions.

- **Now (0.x):** a minor release (0.15 → 0.16) may change the API; a patch release (0.15.0 → 0.15.1) never does. Every change that breaks code is in the [CHANGELOG](CHANGELOG.md) with what to write instead, and the [migration guide](MIGRATING.md) collects them.
- **From 1.0:** minor releases (1.1, 1.2) add things and never break code that compiles against an earlier 1.x. A change that breaks code waits for 2.0. Where it can, the new form arrives first in a 1.x release with the old one deprecated beside it, so you can move over before 2.0. A major release comes at most once a year, with a migration guide.
- **Renames:** a renamed item keeps working, deprecated, for at least one minor release.

## What the promise covers

Covered: every public item of `ferrokit` documented on [docs.rs](https://docs.rs/ferrokit) (also listed in [API.md](crates/ferrokit/API.md)), and `cargo ferrokit`'s commands, flags and `[package.metadata.ferrokit]` settings.

Not covered:

- **Items that name AppKit or objc2 types**, the escape hatches: `WindowRef::ns` and `from_ns`, `native(..)`, `TableModel::table_view`, `TextModel::text_view`, `Color::ns` and `from_ns`, `Font::ns`, the `Stored` trait and the view lookups in `testing` (which return `NSView`s). They follow objc2's versions: when Ferrokit moves to a new objc2-app-kit, they may change in a minor release, and the CHANGELOG says so. The everyday API names no AppKit types, so most apps never touch them.
- **`testing::probes`**: Ferrokit's own counters and switches for its checks.
- **`ferrokit-core`'s own API.** Apps use what `ferrokit` re-exports.
- **Hidden items** (`#[doc(hidden)]`): they're public only because Ferrokit's crates share them.
- **What macOS draws.** Controls are AppKit's, so their look follows the system: a new macOS can change a button's shape or a table's spacing, and that isn't a Ferrokit change.
- **Text that isn't API:** error messages, log lines and `cargo ferrokit`'s output.

## macOS

- **Supported:** macOS 15 or later, which today means macOS 15 and 26. At least the current macOS and the one before are always supported.
- **The minimum rises only in a minor release**, announced in the CHANGELOG one release ahead, and only past versions Apple no longer updates.
- **New macOS versions:** the self-checks run on each beta from the first. What a release breaks (macOS 27 turned column browser cells into text cells, for instance) is fixed in a patch release: before the public release where possible, and within two weeks of it otherwise.
- **Checked on:** macOS 15 and the current release in CI on every change, in release and debug builds, and the newest beta locally. Apple Silicon is checked; Intel builds but isn't run regularly ([#6](https://github.com/lhassa8/ferrokit/issues/6)).

## Rust

The latest stable Rust is tested. The minimum (`rust-version`, now 1.88) rises only in a minor release.

## Fixes

| What | When |
| --- | --- |
| A security issue | A reply within 3 days, and a patch release within 14 days of confirming it. |
| Something a new macOS breaks | A patch release within 2 weeks of that macOS's release, sooner for betas where possible. |
| A regression (worked before, doesn't now) | The next patch release, usually within a week. |
| Other bugs and requests | As time allows. Issues that say what you're building weigh most ([ROADMAP](ROADMAP.md)). |

Fixes go into the latest release only: before 1.0, the newest 0.x; from 1.0, the newest 1.x. After a 2.0, the last 1.x gets security fixes and macOS fixes for six months.

## Reporting

- **Bugs and requests:** [GitHub issues](https://github.com/lhassa8/ferrokit/issues). A small example, the macOS version and the output of `cargo ferrokit check` help most.
- **Security issues:** report them privately, not in a public issue. See [SECURITY.md](SECURITY.md).

## Failures

An Objective-C exception or a panic in code AppKit calls is caught where it came in, reported (`App::on_error`) and survived, never silently dropped. Every change runs the self-checks: about 550 checks across 18 example apps, through the real controls (`scripts/check-all.sh`).
