#!/bin/sh
# Builds a release binary with cargo and packages it as a signed macOS .app.
#
# Usage:
#   scripts/package-app.sh -p <package> (--example <name> | --bin <name>) [options]
#
# Options:
#   --name <display name>   App name (default: the binary name). The bundle is "<name>.app".
#   --id <bundle id>        CFBundleIdentifier (default: dev.ferrokit.<binary>).
#   --version <x.y.z>       Version (default: the package's version from Cargo.toml).
#   --build <n>             CFBundleVersion (default: the version).
#   --icon <file>           .icns, or a square .png (1024 px best), converted to .icns.
#   --min-macos <v>         LSMinimumSystemVersion (default: 15.0).
#   --category <uti>        LSApplicationCategoryType (e.g. public.app-category.developer-tools).
#   --sign <identity>       Code-signing identity (default: "-", ad hoc). A Developer ID
#                           identity also gets a secure timestamp, as notarisation needs.
#   --entitlements <file>   Entitlements plist for codesign.
#   --universal             arm64 + x86_64 (needs: rustup target add x86_64-apple-darwin).
#   --zip                   Also write "<name>.zip" next to the app (ditto, for sharing).
#   --out <dir>             Output folder (default: target/apps).
#
# Signing uses the hardened runtime. Notarisation (xcrun notarytool submit, then
# xcrun stapler staple) is the developer's step: it needs an Apple account.
set -eu
cd "$(dirname "$0")/.."

die() { echo "package-app: $*" >&2; exit 1; }

package= kind= target= name= id= version= build= icon= min=15.0 category= identity=- entitlements= universal= zip= out=target/apps
while [ $# -gt 0 ]; do
  case "$1" in
    -p|--package) package=$2; shift 2 ;;
    --example) kind=example; target=$2; shift 2 ;;
    --bin) kind=bin; target=$2; shift 2 ;;
    --name) name=$2; shift 2 ;;
    --id) id=$2; shift 2 ;;
    --version) version=$2; shift 2 ;;
    --build) build=$2; shift 2 ;;
    --icon) icon=$2; shift 2 ;;
    --min-macos) min=$2; shift 2 ;;
    --category) category=$2; shift 2 ;;
    --sign) identity=$2; shift 2 ;;
    --entitlements) entitlements=$2; shift 2 ;;
    --universal) universal=1; shift ;;
    --zip) zip=1; shift ;;
    --out) out=$2; shift 2 ;;
    -h|--help) sed -n '2,25p' "$0"; exit 0 ;;
    *) die "unknown option $1 (see --help)" ;;
  esac
done
[ -n "$package" ] && [ -n "$target" ] || die "need -p <package> and --example <name> or --bin <name>"
[ -n "$name" ] || name=$target
[ -n "$id" ] || id="dev.ferrokit.$(echo "$target" | tr '_' '-')"
if [ -z "$version" ]; then
  version=$(cargo metadata --no-deps --format-version 1 | python3 -c '
import json, sys
pkg = [p for p in json.load(sys.stdin)["packages"] if p["name"] == sys.argv[1]]
print(pkg[0]["version"] if pkg else "")' "$package")
  [ -n "$version" ] || die "no package $package in this workspace"
fi
[ -n "$build" ] || build=$version
[ -z "$entitlements" ] || [ -f "$entitlements" ] || die "no entitlements file $entitlements"

# Build: one binary per architecture, merged with lipo for --universal.
triples="aarch64-apple-darwin"
[ -z "$universal" ] || triples="aarch64-apple-darwin x86_64-apple-darwin"
installed=$(rustup target list --installed 2>/dev/null || true)
binaries=
for t in $triples; do
  echo "$installed" | grep -qx "$t" || die "Rust target $t isn't installed: rustup target add $t"
  cargo build --release -p "$package" "--$kind" "$target" --target "$t"
  dir="target/$t/release"
  [ "$kind" = example ] && dir="$dir/examples"
  binaries="$binaries $dir/$target"
done

app="$out/$name.app"
# Replace, never update in place: macOS kills a binary rewritten under a cached signature.
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
if [ -z "$universal" ]; then
  cp $binaries "$app/Contents/MacOS/$target"
else
  # shellcheck disable=SC2086
  lipo -create $binaries -output "$app/Contents/MacOS/$target"
fi

icon_key=
if [ -n "$icon" ]; then
  [ -f "$icon" ] || die "no icon file $icon"
  case "$icon" in
    *.icns) cp "$icon" "$app/Contents/Resources/AppIcon.icns" ;;
    *.png)
      set_dir=$(mktemp -d)/AppIcon.iconset
      mkdir -p "$set_dir"
      for s in 16 32 128 256 512; do
        sips -z $s $s "$icon" --out "$set_dir/icon_${s}x${s}.png" >/dev/null
        d=$((s * 2))
        sips -z $d $d "$icon" --out "$set_dir/icon_${s}x${s}@2x.png" >/dev/null
      done
      iconutil -c icns "$set_dir" -o "$app/Contents/Resources/AppIcon.icns"
      rm -rf "$(dirname "$set_dir")"
      ;;
    *) die "icon must be .icns or .png" ;;
  esac
  icon_key="  <key>CFBundleIconFile</key><string>AppIcon</string>"
fi
category_key=
[ -z "$category" ] || category_key="  <key>LSApplicationCategoryType</key><string>$category</string>"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleIdentifier</key><string>$id</string>
  <key>CFBundleName</key><string>$name</string>
  <key>CFBundleDisplayName</key><string>$name</string>
  <key>CFBundleExecutable</key><string>$target</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$build</string>
  <key>LSMinimumSystemVersion</key><string>$min</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSPrincipalClass</key><string>NSApplication</string>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
$icon_key
$category_key
</dict></plist>
PLIST
plutil -lint -s "$app/Contents/Info.plist"
printf 'APPL????' > "$app/Contents/PkgInfo"

# Sign: hardened runtime; a real identity also gets a secure timestamp.
set -- --force --options runtime --sign "$identity"
[ "$identity" = - ] || set -- "$@" --timestamp
[ -z "$entitlements" ] || set -- "$@" --entitlements "$entitlements"
codesign "$@" "$app"
codesign --verify --strict --deep "$app"

if [ -n "$zip" ]; then
  rm -f "$out/$name.zip"
  ditto -c -k --sequesterRsrc --keepParent "$app" "$out/$name.zip"
fi
echo "$app"
