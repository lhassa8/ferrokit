# ferrokit-build

Compiles a Mac app's Swift code from `build.rs` and links it into the app, so Rust can call Swift, including SwiftUI views, through `@_cdecl` functions. Part of [Ferrokit](https://github.com/lhassa8/ferrokit), the native macOS GUI framework for Rust.

```toml
[build-dependencies]
ferrokit-build = "0.8"
```

```rust
// build.rs
fn main() {
    ferrokit_build::swift("swift").compile();
}
```

- **Sources:** every `.swift` file under `swift/` (subfolders too) becomes one Swift module, named after the package (`my-app` becomes `MyApp`).
- **Links what it needs:** the Swift runtime and the frameworks your Swift imports (SwiftUI, Charts…) are linked automatically. They come with macOS, so nothing is copied into the app.
- **Follows the build:** the target architecture (Apple silicon, Intel and universal builds), the Cargo profile's optimisation and debug info, and `MACOSX_DEPLOYMENT_TARGET` (15.0 by default).
- **Options:** `.module(..)`, `.min_macos(..)`, `.swift_version("6")`, `.flag(..)`; `try_compile()` returns the error instead of stopping the build.
- **Errors:** `swiftc`'s own errors show in the build output.
- **Toolchain:** `swiftc` comes with the Command Line Tools (`xcode-select --install`). No Xcode project is needed.

To show a SwiftUI view in a Ferrokit window, return an `NSHostingView` from Swift and wrap it with `ferrokit::native`. Ferrokit's guide, [chapter 14](https://github.com/lhassa8/ferrokit/blob/main/crates/ferrokit/guide/14-custom-native-views.md#swiftui-views), has the recipe. A bridge that needs less glue is being designed in [issue #7](https://github.com/lhassa8/ferrokit/issues/7).

macOS only. License: MIT OR Apache-2.0.
