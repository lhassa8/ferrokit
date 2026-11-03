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
