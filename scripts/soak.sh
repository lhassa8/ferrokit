#!/bin/sh
# Usage: scripts/soak.sh <example> [minutes] [seed]
# Runs an app's soak test (random user actions; see ferrokit::testing::soak)
# and summarises it: crashes (exit code and crash reports), exceptions and
# assertions AppKit logged, hangs, and the footprint over time.
# Examples with a soak: habits, sql_browser. The log is kept in target/soak/.
# The app takes the front at the start and pauses while you use another one.
set -u
cd "$(dirname "$0")/.."
example=${1:?usage: scripts/soak.sh <example> [minutes] [seed]}
minutes=${2:-30}
seed=${3:-}
cargo build --release -p ferrokit --example "$example" 2>&1 | grep -E '^(error|warning)' && exit 1
mkdir -p target/soak
log="target/soak/$example-$(date +%Y%m%d-%H%M%S).log"
start=$(date +%s)
marker=$(mktemp)
echo "soak: $example for $minutes min${seed:+ (seed $seed)}; log $log"
# A watchdog well past the run's end, in case it hangs for good.
limit=$(( minutes * 60 + 300 ))
env SOAK="$minutes" ${seed:+SOAK_SEED="$seed"} "target/release/examples/$example" >"$log" 2>&1 &
pid=$!
( sleep "$limit" && kill -9 "$pid" 2>/dev/null && echo "[soak] killed after $limit s" >>"$log" ) &
watchdog=$!
wait "$pid"
code=$?
kill "$watchdog" 2>/dev/null
wait "$watchdog" 2>/dev/null
echo
grep '^\[soak\] start' "$log"
grep '^\[soak\] sample' "$log" | awk '{ printf "  %5s min  %5s MB  nodes %s  signals %s  windows %s\n", $3, $6, $9, $11, $13 }'
grep -E '^\[soak\] (growth|done|last|hang|killed|paused)' "$log" | uniq
problems=$(grep -cE '\*\*\* |[Ee]xception|[Aa]ssertion failure|panicked' "$log")
crash=$(find "$HOME/Library/Logs/DiagnosticReports" -name "$example*" -newer "$marker" 2>/dev/null | head -1)
rm -f "$marker"
echo
echo "exit code $code; $problems exception or assertion lines${crash:+; crash report $crash}"
if [ "$problems" -gt 0 ]; then
  grep -E '\*\*\* |[Ee]xception|[Aa]ssertion failure|panicked' "$log" | sort | uniq -c | sort -rn | head -10
fi
[ "$code" -eq 0 ] && [ "$problems" -eq 0 ] && [ -z "$crash" ] && echo "soak passed" && exit 0
echo "soak FAILED"
exit 1
