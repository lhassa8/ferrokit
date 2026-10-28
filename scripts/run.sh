#!/bin/sh
# Build the spike and launch it as a minimal .app bundle (needed for a proper
# bundle id, menu bar name, IME and VoiceOver behaviour).
set -e
cd "$(dirname "$0")/.."
cargo build "$@"
profile=debug
case " $* " in *" --release "*) profile=release ;; esac

app=target/NativeSpike.app
mkdir -p "$app/Contents/MacOS"
# Replace, never overwrite in place: macOS SIGKILLs a binary rewritten under a cached signature.
rm -f "$app/Contents/MacOS/NativeSpike"
cp "target/$profile/nativekit-spike" "$app/Contents/MacOS/NativeSpike"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>dev.spike.nativekit</string>
  <key>CFBundleName</key><string>NativeSpike</string>
  <key>CFBundleExecutable</key><string>NativeSpike</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
# Run the binary directly so stdout/stderr ([native]/[spike] logs) stay in this terminal.
exec "$app/Contents/MacOS/NativeSpike"
