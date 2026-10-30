# cargo-ferrokit

Make, run, check, bundle and notarize native Mac apps built with
[`appkit-ui`](https://crates.io/crates/appkit-ui).

```bash
cargo install cargo-ferrokit
```

```bash
cargo ferrokit new my-app
```

```bash
cd my-app && cargo ferrokit run
```

| Command | What it does |
| --- | --- |
| `new <path> [--document]` | A new app: a to-do list, or a text document app, with self-checks. |
| `run [--release]` | Builds, bundles and runs the app (its name, icon and identity), output in the terminal. |
| `check` | Clippy (warnings are errors), tests, a bundle, the app's self-checks (`CHECK=all`), the signature. |
| `bundle [--universal] [--zip]` | A signed `.app` with the hardened runtime (ad hoc, or your Developer ID). |
| `notarize [--dry-run]` | Developer ID signing, Apple's notary service, stapling, a zip to share. |

Settings live in the app's `Cargo.toml`:

```toml
[package.metadata.ferrokit]
name = "My App"
id = "com.example.myapp"
icon = "icon.png"          # square PNG (1024 px) or .icns
min-macos = "15.0"
category = "public.app-category.productivity"
languages = ["de", "fr"]   # its translations, besides English
sign = "Developer ID Application: Your Name (TEAMID)"
notary-profile = "my-profile"   # xcrun notarytool store-credentials my-profile …

[[package.metadata.ferrokit.documents]]
name = "Text"
extensions = ["txt", "md"]
```

macOS only. It runs Apple's tools (`codesign`, `plutil`, `sips`,
`iconutil`, `lipo`, `ditto`, `xcrun notarytool`, `stapler`, `spctl`), which
come with the Command Line Tools.

License: MIT OR Apache-2.0.
