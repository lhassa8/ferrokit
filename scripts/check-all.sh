#!/bin/sh
# Runs every check in the workspace: build, clippy, unit tests, docs, and
# each app's unattended suite (CHECK=all), then prints a summary. Exits
# non-zero if anything failed.
#
# Usage: scripts/check-all.sh [--no-iced] [--no-lint]
#   --no-iced   skip the iced-appkit checklist (it bundles an .app first)
#   --no-lint   skip clippy, tests and docs (suites only)
#
# Logs go to a temporary folder, or to $CHECK_ALL_LOGS if set (CI uploads it).
# The suites open real windows: run it in a logged-in session with the
# display awake. Each suite prints "[check] <name> PASS|FAIL ..." lines.
set -u
cd "$(dirname "$0")/.."

iced=1 lint=1
for a in "$@"; do
  case "$a" in
    --no-iced) iced= ;;
    --no-lint) lint= ;;
    -h|--help) sed -n '2,13p' "$0"; exit 0 ;;
    *) echo "check-all: unknown option $a" >&2; exit 2 ;;
  esac
done

log=${CHECK_ALL_LOGS:-$(mktemp -d)/check-all}
mkdir -p "$log"
summary=
failed=0

# step <name> <command…>: runs a command, records pass or fail.
step() {
  name=$1; shift
  printf '%-22s ' "$name"
  start=$(date +%s)
  if "$@" >"$log/$name.log" 2>&1; then
    printf 'ok     (%ss)\n' $(( $(date +%s) - start ))
    summary="$summary\n  ok    $name"
  else
    printf 'FAILED (%ss): %s\n' $(( $(date +%s) - start )) "$log/$name.log"
    tail -5 "$log/$name.log" | sed 's/^/    /'
    summary="$summary\n  FAIL  $name"
    failed=$((failed + 1))
  fi
}

# suite <name> <env> <binary> [args]: an app's self-checks; also counts checks.
suite() {
  name=$1 env=$2; shift 2
  printf '%-22s ' "$name"
  start=$(date +%s)
  # A hung suite fails after 10 minutes instead of blocking the run; its
  # call stacks are sampled first, to show where it hung.
  env "$env" "$@" >"$log/$name.log" 2>&1 &
  pid=$!
  (
    sleep 600
    if kill -0 "$pid" 2>/dev/null; then
      sample "$pid" 3 -file "$log/$name.hang.txt" >/dev/null 2>&1
      kill "$pid" 2>/dev/null
      echo "[check] watchdog FAIL killed after 600 s; stacks in $name.hang.txt" >>"$log/$name.log"
    fi
  ) &
  dog=$!
  wait "$pid"
  code=$?
  kill "$dog" 2>/dev/null
  wait "$dog" 2>/dev/null
  # Died on a signal (a crash, not the watchdog): keep macOS's crash report.
  if [ "$code" -gt 128 ] && [ ! -f "$log/$name.hang.txt" ]; then
    bin=$(basename "$1")
    report=$(ls -t "$HOME/Library/Logs/DiagnosticReports/$bin"*.ips 2>/dev/null | head -1)
    for _ in 1 2 3 4 5; do [ -n "$report" ] && break; sleep 2; report=$(ls -t "$HOME/Library/Logs/DiagnosticReports/$bin"*.ips 2>/dev/null | head -1); done
    [ -n "$report" ] && cp "$report" "$log/$name.crash.ips"
  fi
  pass=$(grep -c '^\[check\] .* PASS' "$log/$name.log")
  fail=$(grep -c '^\[check\] .* FAIL' "$log/$name.log")
  if [ "$code" -eq 0 ] && [ "$fail" -eq 0 ] && [ "$pass" -gt 0 ]; then
    printf 'ok     %3s checks (%ss)\n' "$pass" $(( $(date +%s) - start ))
    summary="$summary\n  ok    $name: $pass checks"
  else
    printf 'FAILED %3s of %s checks, exit %s (%ss)\n' "$fail" $((pass + fail)) "$code" $(( $(date +%s) - start ))
    grep '^\[check\] .* FAIL' "$log/$name.log" | sed 's/^/    /'
    echo "    last lines:"
    tail -6 "$log/$name.log" | sed 's/^/      /'
    if [ -f "$log/$name.crash.ips" ]; then
      echo "    crashed; the faulting thread:"
      python3 - "$log/$name.crash.ips" <<'PY' 2>/dev/null | head -25 | sed 's/^/      /'
import json, sys
text = open(sys.argv[1]).read()
body = json.loads(text[text.index("\n") + 1:])
ex = body.get("exception", {})
print("exception:", ex.get("type"), ex.get("signal"), body.get("termination", {}).get("indicator", ""))
threads = body.get("threads", [])
images = body.get("usedImages", [])
for t in threads:
    if t.get("triggered"):
        for f in t.get("frames", [])[:20]:
            img = images[f["imageIndex"]].get("name", "?") if "imageIndex" in f and f["imageIndex"] < len(images) else "?"
            print(img, f.get("symbol", hex(f.get("imageOffset", 0))))
PY
    fi
    if [ -f "$log/$name.hang.txt" ]; then
      echo "    main thread when it hung:"
      grep -A40 'Thread_.*com.apple.main-thread' "$log/$name.hang.txt" | grep -E 'ui_checklist|appkit_ui|AppKit|\]' | head -25 | sed 's/^/      /'
    fi
    summary="$summary\n  FAIL  $name: $fail failed, exit $code"
    failed=$((failed + 1))
  fi
}

step build cargo build --release --workspace --examples
if [ -n "$lint" ]; then
  step clippy cargo clippy --release --workspace --examples -- -D warnings
  step tests cargo test --workspace --release
  step docs env RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
fi

ex=target/release/examples
suite ui_checklist CHECK=all "$ex/ui_checklist"
suite settings CHECK=all "$ex/settings"
suite native_csv CHECK=all "$ex/native_csv"
suite log_viewer CHECK=all "$ex/log_viewer"
suite hands_on CHECK=drop "$ex/hands_on"
suite notes CHECK=all "$ex/notes"
suite gallery CHECK=all "$ex/gallery"
suite sql_browser CHECK=all "$ex/sql_browser"
suite sql_browser_de CHECK=l10n "$ex/sql_browser" -AppleLanguages "(de)"
suite cargo_ferrokit FERROKIT_E2E=1 scripts/check-ferrokit.sh
if [ -n "$iced" ]; then
  step iced_bundle scripts/bundle-example.sh checklist
  suite iced_checklist CHECK=all target/checklist.app/Contents/MacOS/checklist
fi

printf '\nSummary (logs in %s):%b\n' "$log" "$summary"
if [ "$failed" -gt 0 ]; then
  echo "$failed failed"
  exit 1
fi
echo "all passed"
