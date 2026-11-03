#!/bin/bash
# Records an example's `DEMO=1` tour as screen video. Each
# "[demo] frame x y w h" line it prints starts a new clip of that part of
# the screen; "[demo] done" ends the last one and quits the app.
#
#   scripts/record-demo.sh [--shots] <example> <out-dir> [VAR=value ...] [-- app arguments]
#
# Clips go to <out-dir>/<example>-<n>.mov (60 fps, Retina size). With
# --shots, no video: a lossless screenshot of the window every second
# instead (<example>-shot-<n>.png, with its shadow), to pick from. The app
# takes the front: leave the Mac alone while it runs. Needs Screen
# Recording permission for the terminal.
set -euo pipefail
cd "$(dirname "$0")/.."
shots=""
if [[ $1 == --shots ]]; then
  shots=1
  shift
fi
ex=$1 out=$2
shift 2
vars=() args=()
while (($#)); do
  if [[ $1 == -- ]]; then
    shift
    args=("$@")
    break
  fi
  vars+=("$1")
  shift
done
mkdir -p "$out"
cargo build -q --release -p ferrokit --example "$ex"
log="$out/$ex.log"
env DEMO=1 ${vars[@]+"${vars[@]}"} "target/release/examples/$ex" ${args[@]+"${args[@]}"} >"$log" 2>&1 &
app=$!
rec="" n=0 seen=0 done="" win="" shot=0 last=0
stop() {
  if [[ -n $rec ]]; then
    kill -INT "$rec" 2>/dev/null || true
    wait "$rec" 2>/dev/null || true
    rec=""
  fi
}
while [[ -z $done ]] && kill -0 "$app" 2>/dev/null; do
  total=$(wc -l <"$log")
  while ((seen < total)); do
    seen=$((seen + 1))
    line=$(sed -n "${seen}p" "$log")
    case "$line" in
      "[demo] window "*)
        win=${line#"[demo] window "}
        ;;
      "[demo] frame "*)
        [[ -n $shots ]] && continue
        stop
        read -r x y w h <<<"${line#"[demo] frame "}"
        n=$((n + 1))
        screencapture -v -x -R"$x,$y,$w,$h" "$out/$ex-$n.mov" </dev/null &
        rec=$!
        ;;
      "[demo] done")
        stop
        done=1
        break
        ;;
    esac
  done
  if [[ -n $shots && -n $win && -z $done && $SECONDS -gt $last ]]; then
    last=$SECONDS
    shot=$((shot + 1))
    screencapture -x -l"$win" "$out/$ex-shot-$(printf %02d $shot).png" || true
  fi
  sleep 0.05
done
stop
kill "$app" 2>/dev/null || true
wait "$app" 2>/dev/null || true
ls "$out/$ex"-* | grep -v '\.log$' || true
