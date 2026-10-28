#!/bin/sh
# Usage: scripts/bundle-example.sh <example> [--lowres]
# Builds a release example and wraps it in target/<example>.app (bundle id
# dev.spike.nativekit, so screen-recording and accessibility grants carry over).
# --lowres sets NSHighResolutionCapable false: the app runs at 1x on a Retina screen.
set -e
cd "$(dirname "$0")/.."
cargo build --release -p iced-appkit --example "$1"
app="target/$1.app"
hires=true; [ "$2" = "--lowres" ] && hires=false
mkdir -p "$app/Contents/MacOS"
# Replace, never overwrite in place: macOS SIGKILLs a binary rewritten under a cached signature.
rm -f "$app/Contents/MacOS/$1"
cp "target/release/examples/$1" "$app/Contents/MacOS/$1"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>dev.spike.nativekit</string>
  <key>CFBundleName</key><string>$1</string>
  <key>CFBundleExecutable</key><string>$1</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>15.0</string>
  <key>NSHighResolutionCapable</key><$hires/>
</dict></plist>
PLIST
echo "$app/Contents/MacOS/$1"
