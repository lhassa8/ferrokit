#!/bin/sh
# Runs every check in the workspace: build, clippy, unit tests, docs, and
# each app's unattended suite (CHECK=all), then prints a summary. Exits
# non-zero if anything failed.
#
# Usage: scripts/check-all.sh [--no-lint] [--debug]
#   --no-lint   skip clippy, tests and docs (suites only)
#   --debug     build and run the suites as debug builds, where objc2
#               checks every message's types and every protocol's required
#               methods (slower; timing budgets are relaxed in debug)
#
# Logs go to a temporary folder, or to $CHECK_ALL_LOGS if set (CI uploads it).
# The suites open real windows: run it in a logged-in session with the
# display awake. Each suite prints "[check] <name> PASS|FAIL|SKIP ..." lines.
set -u
cd "$(dirname "$0")/.."

lint=1 debug=
for a in "$@"; do
  case "$a" in
    --debug) debug=1 ;;
    --no-iced) ;; # (the Iced checklist is retired; accepted for old scripts)
    --no-lint) lint= ;;
    -h|--help) sed -n '2,16p' "$0"; exit 0 ;;
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
  # Crash reports newer than this mark are this suite's (a child process
  # it launched can crash while the suite itself passes).
  mark="$log/$name.mark"
  : >"$mark"
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
  bin=$(basename "$1")
  sleep 2 # (ReportCrash writes a report a moment after the crash)
  for report in $(find "$HOME/Library/Logs/DiagnosticReports" -name "$bin-*.ips" -newer "$mark" 2>/dev/null); do
    echo "[check] no-crash-reports FAIL a $bin process crashed: $report" >>"$log/$name.log"
    [ -f "$log/$name.crash.ips" ] || cp "$report" "$log/$name.crash.ips"
  done
  pass=$(grep -c '^\[check\] .* PASS' "$log/$name.log")
  fail=$(grep -c '^\[check\] .* FAIL' "$log/$name.log")
  # Checks that can't run in this session (window-server input without
  # Accessibility trust) say SKIP, and are reported, not failed.
  skip=$(grep -c '^\[check\] .* SKIP' "$log/$name.log")
  skipped=""
  [ "$skip" -gt 0 ] && skipped=", $skip skipped"
  if [ "$code" -eq 0 ] && [ "$fail" -eq 0 ] && [ "$pass" -gt 0 ]; then
    printf 'ok     %3s checks%s (%ss)\n' "$pass" "$skipped" $(( $(date +%s) - start ))
    summary="$summary\n  ok    $name: $pass checks$skipped"
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
      grep -A40 'Thread_.*com.apple.main-thread' "$log/$name.hang.txt" | grep -E 'ui_checklist|ferrokit|AppKit|\]' | head -25 | sed 's/^/      /'
    fi
    summary="$summary\n  FAIL  $name: $fail failed, exit $code"
    failed=$((failed + 1))
  fi
}

if [ -n "$debug" ]; then
  step build cargo build --workspace --examples
else
  step build cargo build --release --workspace --examples
fi
if [ -n "$lint" ]; then
  step clippy cargo clippy --release --workspace --examples -- -D warnings
  step tests cargo test --workspace --release
  step docs env RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
  step api_md scripts/api-md.sh --check
fi

ex=target/release/examples
[ -n "$debug" ] && ex=target/debug/examples
suite ui_checklist CHECK=all "$ex/ui_checklist"
suite settings CHECK=all "$ex/settings"
suite native_csv CHECK=all "$ex/native_csv"
suite log_viewer CHECK=all "$ex/log_viewer"
suite hands_on CHECK=drop "$ex/hands_on"
suite notes CHECK=all "$ex/notes"
suite journal CHECK=all "$ex/journal"
suite gallery CHECK=all "$ex/gallery"
suite styles CHECK=all "$ex/styles"
suite materials CHECK=all "$ex/materials"
suite motion CHECK=all "$ex/motion"
suite components CHECK=all "$ex/components"
suite media CHECK=all "$ex/media"
suite habits CHECK=all "$ex/habits"
suite input CHECK=all "$ex/input"
suite values CHECK=all "$ex/values"
suite native_views CHECK=all "$ex/native_views"
suite sql_browser CHECK=all "$ex/sql_browser"
suite sql_browser_de CHECK=l10n "$ex/sql_browser" -AppleLanguages "(de)"
# The notes example in the App Sandbox (signed with the entitlements
# `cargo ferrokit bundle` gives `sandbox = true`): documents, recents and
# restoring them work there too.
if [ -z "$debug" ]; then
  step notes_sandbox_bundle scripts/package-app.sh -p ferrokit --example notes --name "Notes Sandboxed" --id dev.ferrokit.notes-sandboxed --entitlements scripts/sandbox.entitlements
  suite notes_sandboxed CHECK=all "target/apps/Notes Sandboxed.app/Contents/MacOS/notes"
fi
# (cargo ferrokit's templates build their own debug apps already.)
[ -z "$debug" ] && suite cargo_ferrokit FERROKIT_E2E=1 scripts/check-ferrokit.sh

printf '\nSummary (logs in %s):%b\n' "$log" "$summary"
# On GitHub the suites' step may be allowed to fail, so its result has to
# show without opening the log: a summary on the run's page, and an error
# annotation per failed suite.
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  { printf '### Suites%s\n\n```' "${debug:+ (debug builds)}"; printf '%b\n' "$summary"; printf '```\n'; } >>"$GITHUB_STEP_SUMMARY"
  printf '%b\n' "$summary" | grep '^  FAIL' | while read -r _ rest; do echo "::error title=Suite failed::$rest"; done
fi
if [ "$failed" -gt 0 ]; then
  echo "$failed failed"
  exit 1
fi
echo "all passed"
