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
other=x86_64-apple-darwin
[ "$(uname -m)" = x86_64 ] && other=aarch64-apple-darwin
if ! rustup target list --installed 2>/dev/null | grep -qx "$other"; then
  echo "[check] app-store-dry-run SKIP the Rust target $other isn't installed (rustup target add $other)"
elif pkg=$(cd e2e-app && cargo ferrokit bundle --app-store --dry-run --build 7 2>"$dir/store.log"); then
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
# Updates through Sparkle, when it's here (FERROKIT_SPARKLE, or unpacked
# in target/sparkle-*): version 1.0.0 is installed, 1.1.0 goes in a feed
# served on this Mac; an update signed with the wrong key is refused, the
# right one installs when the app quits. Throwaway keys (openssl), never
# the keychain.
sparkle=${FERROKIT_SPARKLE:-}
if [ -z "$sparkle" ]; then
  for d in "$root"/target/sparkle-*; do [ -d "$d/Sparkle.framework" ] && sparkle=$d; done
fi
update_check() {
  # $1: "plain", or "sandboxed" (Sparkle installs, and downloads without
  # network access, through its XPC services).
  id=com.example.ferrokit-update-test
  u="$dir/update-$1"
  port=$(python3 -I -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
  mkdir -p "$u/keys" "$u/feed" "$u/installed"
  for k in good bad; do
    openssl genpkey -algorithm ed25519 -out "$u/keys/$k.pem" 2>/dev/null || return 2
    openssl pkey -in "$u/keys/$k.pem" -outform DER | tail -c 32 | base64 >"$u/keys/$k.txt"
  done
  public=$(openssl pkey -in "$u/keys/good.pem" -pubout -outform DER | tail -c 32 | base64)
  app_dir=e2e-update-$1
  cargo ferrokit new "$app_dir" --name "Update Test" --id "$id" --ferrokit-path "$root/crates/ferrokit" >/dev/null
  cat >"$app_dir/src/main.rs" <<'RS'
use ferrokit::*;

fn main() {
    App::new("Update Test")
        .window(Window::new("Update Test").view(text(concat!("Version ", env!("CARGO_PKG_VERSION")))))
        .on_launch(|| {
            let menu = testing::app_menu_titles();
            let ok = menu.iter().position(|t| t == "Check for Updates…") == Some(1);
            eprintln!("[update] menu {} {menu:?}", if ok { "ok" } else { "missing" });
            updates::on_event(|e| {
                eprintln!("[update] {e:?}");
                if !matches!(e, UpdateEvent::Found { .. }) {
                    quit();
                }
            });
            updates::set_automatic_checks(true);
            updates::set_automatic_downloads(true);
            updates::check_in_background();
        })
        .run();
}
RS
  printf '\n[package.metadata.ferrokit.updates]\nfeed = "http://127.0.0.1:%s/appcast.xml"\npublic-key = "%s"\nsparkle = "%s"\n' "$port" "$public" "$sparkle" >>"$app_dir/Cargo.toml"
  if [ "$1" = sandboxed ]; then
    sed -i '' 's/^\[package.metadata.ferrokit\]$/[package.metadata.ferrokit]\
sandbox = true/' "$app_dir/Cargo.toml"
  fi
  bundled="$CARGO_TARGET_DIR/release/bundle/Update Test.app"
  (cd "$app_dir" && sed -i '' 's/^version = .*/version = "1.0.0"/' Cargo.toml && cargo ferrokit bundle) >"$u/update.log" 2>&1 || { echo "[check] sparkle-update-$1 FAIL bundling or the feed"; tail -20 "$u/update.log"; return 1; }
  ditto "$bundled" "$u/installed/Update Test.app"
  (cd "$app_dir" && sed -i '' 's/^version = "1.0.0"/version = "1.1.0"/' Cargo.toml && cargo ferrokit bundle &&
    cargo ferrokit appcast --key-file "$u/keys/bad.txt" --out "$u/bad" &&
    cargo ferrokit appcast --key-file "$u/keys/good.txt" --out "$u/good") >>"$u/update.log" 2>&1 || { echo "[check] sparkle-update-$1 FAIL bundling or the feed"; tail -20 "$u/update.log"; return 1; }
  python3 -I -m http.server "$port" --bind 127.0.0.1 --directory "$u/feed" >"$u/http.log" 2>&1 &
  server=$!
  installed="$u/installed/Update Test.app"
  # One run of the installed app against the feed in $1; its version once
  # it's $2, or after $3 seconds (Sparkle installs in a second or two).
  run_installed() {
    cp -R "$u/$1/." "$u/feed/"
    "$installed/Contents/MacOS/$app_dir" >"$u/run-$1.log" 2>&1 &
    app=$!
    i=0
    while kill -0 "$app" 2>/dev/null && [ "$i" -lt 60 ]; do sleep 1; i=$((i + 1)); done
    kill "$app" 2>/dev/null || true
    i=0
    while [ "$i" -lt "$3" ]; do
      v=$(plutil -extract CFBundleShortVersionString raw "$installed/Contents/Info.plist" 2>/dev/null || true)
      [ "$v" = "$2" ] && break
      sleep 1; i=$((i + 1))
    done
    echo "$v"
  }
  sleep 1
  after_bad=$(run_installed bad 1.1.0 8)
  after_good=$(run_installed good 1.1.0 30)
  kill "$server" 2>/dev/null || true
  defaults delete "$id" >/dev/null 2>&1 || true
  # (The sandboxed run's container stays: macOS protects it; later runs reuse it.)
  rm -rf "$HOME/Library/Caches/$id" "$HOME/Library/HTTPStorages/$id"
  refused=$(grep -c '^\[update\] Failed' "$u/run-bad.log" || true)
  ready=$(grep -c '^\[update\] ReadyOnQuit { version: "1.1.0" }' "$u/run-good.log" || true)
  menu=$(grep -c '^\[update\] menu ok' "$u/run-good.log" || true)
  if [ "$after_bad" = "1.0.0" ] && [ "$refused" = "1" ] && [ "$after_good" = "1.1.0" ] && [ "$ready" = "1" ] && [ "$menu" = "1" ] &&
    codesign --verify --strict --deep "$installed" 2>/dev/null; then
    echo "[check] sparkle-update-$1 PASS Check for Updates… in the menu; wrong key refused (still 1.0.0); 1.0.0 → 1.1.0 installed on quit, signature valid"
    return 0
  fi
  echo "[check] sparkle-update-$1 FAIL after the wrong key '$after_bad' (refused $refused), after the right one '$after_good' (ready $ready, menu $menu)"
  tail -5 "$u/run-bad.log" "$u/run-good.log" "$u/update.log" "$u/http.log"
  return 1
}
if [ -z "$sparkle" ]; then
  echo "[check] sparkle-update SKIP no Sparkle (set FERROKIT_SPARKLE, or unpack its release in target/sparkle-<version>)"
else
  for variant in plain sandboxed; do
    update_check "$variant" && status=0 || status=$?
    if [ "$status" = 2 ]; then
      echo "[check] sparkle-update-$variant SKIP this openssl can't make Ed25519 keys"
    elif [ "$status" != 0 ]; then
      failed=$((failed + 1))
    fi
  done
fi
# Live preview: a save rebuilds and restarts the app (a new process); a save
# that breaks the build leaves the running app alone, and fixing it
# restarts it again.
cargo ferrokit new e2e-preview --ferrokit-path "$root/crates/ferrokit" >/dev/null
(cd e2e-preview && exec cargo ferrokit preview) >"$dir/preview.log" 2>&1 &
previewer=$!
# Waits up to $2 seconds for line number $1 of a pattern ($3) in the log, and prints its pid.
pid_of() {
  n=0
  while [ "$n" -lt "$2" ]; do
    p=$(grep -E "$3" "$dir/preview.log" | sed -n "$1s/.*(pid \([0-9]*\)).*/\1/p")
    [ -n "$p" ] && { echo "$p"; return 0; }
    sleep 1
    n=$((n + 1))
  done
  return 1
}
first=$(pid_of 1 240 'Previewing') || first=
sleep 2
printf '\n// edited by the preview check\n' >>e2e-preview/src/main.rs
second=$(pid_of 1 180 'Restarted') || second=
first_gone=no
[ -n "$first" ] && ! kill -0 "$first" 2>/dev/null && first_gone=yes
cp e2e-preview/src/main.rs "$dir/main.rs.good"
printf '\nfn broken( {\n' >>e2e-preview/src/main.rs
n=0
while [ "$n" -lt 120 ] && ! grep -q 'build failed' "$dir/preview.log"; do sleep 1; n=$((n + 1)); done
kept=no
[ -n "$second" ] && grep -q 'build failed' "$dir/preview.log" && kill -0 "$second" 2>/dev/null && kept=yes
cp "$dir/main.rs.good" e2e-preview/src/main.rs
third=$(pid_of 2 180 'Restarted') || third=
kill "$previewer" 2>/dev/null || true
for p in $first $second $third; do kill "$p" 2>/dev/null || true; done
if [ -n "$first" ] && [ -n "$second" ] && [ "$first_gone" = yes ] && [ "$kept" = yes ] && [ -n "$third" ] && [ "$third" != "$second" ]; then
  echo "[check] preview-restarts PASS pid $first, restarted as $second after a save; a broken build kept $second running; fixed, restarted as $third"
else
  echo "[check] preview-restarts FAIL first '$first' (gone $first_gone), after a save '$second', kept on a broken build $kept, after the fix '$third'"
  tail -30 "$dir/preview.log"
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
