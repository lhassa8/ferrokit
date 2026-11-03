#!/bin/sh
# Writes crates/ferrokit/API.md, the one-page API list, from rustdoc's JSON
# (on stable, through RUSTC_BOOTSTRAP). `--check` writes it elsewhere and
# fails if the committed one is out of date.
set -eu
cd "$(dirname "$0")/.."
RUSTC_BOOTSTRAP=1 RUSTDOCFLAGS="-Z unstable-options --output-format json" \
  cargo doc -q -p ferrokit -p ferrokit-core --no-deps --target-dir target/apijson
if [ "${1:-}" = "--check" ]; then
  out=$(mktemp)
  python3 scripts/api-md.py target/apijson/doc "$out" >/dev/null
  if cmp -s "$out" crates/ferrokit/API.md; then
    rm -f "$out"
    echo "API.md is current"
  else
    diff -u crates/ferrokit/API.md "$out" | head -20
    rm -f "$out"
    echo "API.md is out of date: run scripts/api-md.sh"
    exit 1
  fi
else
  python3 scripts/api-md.py target/apijson/doc crates/ferrokit/API.md
fi
