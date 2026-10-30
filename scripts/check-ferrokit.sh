#!/bin/sh
# cargo ferrokit end to end: makes an app and a document app from the
# templates (against this checkout's appkit-ui), then runs `cargo ferrokit
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
cargo ferrokit new e2e-app --appkit-ui-path "$root/crates/appkit-ui"
cargo ferrokit new e2e-docs --document --appkit-ui-path "$root/crates/appkit-ui"
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
echo "[check] done, $failed failed"
[ "$failed" -eq 0 ]
