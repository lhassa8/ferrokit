#!/bin/sh
# cargo ferrokit end to end: makes an app and a document app from the
# templates (against this checkout's ferrokit), then runs `cargo ferrokit
# check` on each (clippy, tests, bundle, signature, self-checks).
# The apps share target/ferrokit-e2e, so later runs only rebuild them.
set -eu
cd "$(dirname "$0")/.."
root=$(pwd)
cargo build --release -p cargo-ferrokit
export PATH="$root/target/release:$PATH" CARGO_TARGET_DIR="$root/target/ferrokit-e2e"
dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT
cd "$dir"
cargo ferrokit new e2e-app --ferrokit-path "$root/crates/ferrokit"
cargo ferrokit new e2e-docs --document --ferrokit-path "$root/crates/ferrokit"
# The Rust in the AGENTS.md new apps get compiles (and passes clippy) in the app.
{ echo '#![allow(dead_code)]'; sed -n '/^```rust$/,/^```$/p' e2e-app/AGENTS.md | sed '/^```/d'; } >e2e-app/src/agents_recipe.rs
printf '\nmod agents_recipe;\n' >>e2e-app/src/main.rs
failed=0
for app in e2e-app e2e-docs; do
  # The apps' [check] lines, unindented, so check-all counts them.
  if (cd "$app" && cargo ferrokit check --timeout 120) >"$dir/$app.log" 2>&1; then
    echo "[check] $app-ferrokit-check PASS lint, tests, bundle, signature, self-checks"
  else
    echo "[check] $app-ferrokit-check FAIL"
    tail -20 "$dir/$app.log"
    failed=$((failed + 1))
  fi
  sed -n 's/^ *\(\[check\] [^d].*\)/\1/p' "$dir/$app.log" | sed "s/^\[check\] /[check] $app: /"
done
# The Mac App Store, as far as it goes without Apple's identities: a dry
# run makes a universal, sandboxed app with the build number, and an
# unsigned installer package holding it.
printf 'sandbox = true\ncategory = "public.app-category.productivity"\n' >>e2e-app/Cargo.toml
if pkg=$(cd e2e-app && cargo ferrokit bundle --app-store --dry-run --build 7 2>"$dir/store.log"); then
  app="$(dirname "$pkg")/E2e App.app"
  archs=$(lipo -archs "$app/Contents/MacOS/e2e-app" 2>/dev/null || true)
  build=$(plutil -extract CFBundleVersion raw "$app/Contents/Info.plist" 2>/dev/null || true)
  sandboxed=$(codesign -d --entitlements - --xml "$app" 2>/dev/null | grep -c app-sandbox || true)
  holds=$(pkgutil --payload-files "$pkg" 2>/dev/null | grep -c '^\./E2e App.app/Contents/MacOS/e2e-app$' || true)
  if [ "$archs" = "x86_64 arm64" ] && [ "$build" = "7" ] && [ "$sandboxed" -ge 1 ] && [ "$holds" = "1" ]; then
    echo "[check] app-store-dry-run PASS universal ($archs), build $build, sandboxed, in $(basename "$pkg")"
  else
    echo "[check] app-store-dry-run FAIL archs '$archs', build '$build', sandbox $sandboxed, package holds the app $holds"
    failed=$((failed + 1))
  fi
else
  echo "[check] app-store-dry-run FAIL"
  tail -20 "$dir/store.log"
  failed=$((failed + 1))
fi
# A failing app: multi-word check names are counted, and a panic that stops
# the checks ends the run in seconds, not at the timeout.
cargo ferrokit new e2e-fails --ferrokit-path "$root/crates/ferrokit" >/dev/null
cat >e2e-fails/src/main.rs <<'RS'
use ferrokit::*;

fn main() {
    App::new("Fails")
        .window(Window::new("Fails").view(text("Fails")))
        .on_launch(|| {
            if std::env::var("CHECK").is_ok_and(|v| v == "all") {
                eprintln!("[check] two words PASS fine");
                eprintln!("[check] fails on purpose FAIL as planned");
                panic!("on purpose");
            }
        })
        .run();
}
RS
start=$(date +%s)
(cd e2e-fails && cargo ferrokit check --no-lint --timeout 120) >"$dir/e2e-fails.log" 2>&1 || true
took=$(( $(date +%s) - start ))
if grep -q "stopped reporting checks after a failure (1 passed, 1 failed so far); it hit ferrokit: a panic" "$dir/e2e-fails.log" && [ "$took" -lt 90 ]; then
  echo "[check] check-reports-failures PASS multi-word names counted, stopped after the panic (${took}s with the build)"
else
  echo "[check] check-reports-failures FAIL (${took}s)"
  tail -20 "$dir/e2e-fails.log"
  failed=$((failed + 1))
fi
echo "[check] done, $failed failed"
[ "$failed" -eq 0 ]
