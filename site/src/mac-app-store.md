# Shipping a Rust app to the Mac App Store

The Mac App Store takes an installer package (`.pkg`) of a **sandboxed** app, built for Apple silicon and Intel, signed with your App Store certificates and carrying a provisioning profile. With Ferrokit, `cargo ferrokit bundle --app-store` makes that package from your Cargo project. This is the path Rowboat, a SQLite editor built with Ferrokit, took to App Store submission.

## What you need from Apple

From your Apple Developer account:

- an **Apple Distribution** certificate (signs the app) and a **Mac Installer Distribution** certificate (signs the package; Keychain Access shows it as "3rd Party Mac Developer Installer"), both in your keychain;
- a **Mac App Store provisioning profile** for the app's bundle id (developer.apple.com ▸ Profiles), saved next to the app;
- the app in **App Store Connect**, with the same bundle id.

## The settings

```toml
[package.metadata.ferrokit]
name = "My App"
id = "com.example.myapp"
icon = "icon.png"
sandbox = true
category = "public.app-category.productivity"
app-store-sign = "Apple Distribution: Your Name (TEAMID)"
installer-sign = "3rd Party Mac Developer Installer: Your Name (TEAMID)"
provisioning-profile = "MyApp.provisionprofile"
```

## Build the package

```bash
cargo ferrokit bundle --app-store --build 42
```

It builds a universal app, embeds the profile, signs the app with its sandbox entitlements and the profile's app and team ids (a profile for another app, or for iOS, is refused with the reason), and writes `My App.pkg`. `--build` is the build number (`CFBundleVersion`); each upload needs a higher one. `--dry-run` does all of it without your identities, then prints the signing commands it would run.

Upload the package with Apple's Transporter app.

## Before you submit: run it sandboxed

App Review checks that the app works inside the App Sandbox, where files outside the app's container are reachable only through the open and save panels, drops, and the security-scoped bookmarks Ferrokit keeps for Open Recent. Run the sandboxed build yourself first:

```bash
cargo ferrokit run
```

with `sandbox = true` in `Cargo.toml`. Ferrokit's own example apps run their self-checks in the sandbox too.

## Outside the store

For apps you distribute yourself: `cargo ferrokit notarize` (Developer ID signing, Apple's notary service, stapling), Sparkle updates with `cargo ferrokit appcast`, and crash reports with `App::crash_reports`. All in [Testing and shipping](guide/08-testing-and-shipping.md).
