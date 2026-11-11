#!/bin/sh
# Writes crates/ferrokit/AGENTS.md, the guidance for AI coding agents that
# ships in the crate, from the one `cargo ferrokit new` puts in every app
# (crates/cargo-ferrokit/templates/AGENTS.md): the same text under a header
# of its own. `--check` fails if the committed one is out of date.
set -eu
cd "$(dirname "$0")/.."
out=crates/ferrokit/AGENTS.md
[ "${1:-}" = "--check" ] && out=$(mktemp)
python3 - "$out" <<'PY'
import re, sys
template = open("crates/cargo-ferrokit/templates/AGENTS.md").read()
version = re.search(r'const FERROKIT_VERSION: &str = "([^"]+)"', open("crates/cargo-ferrokit/src/main.rs").read()).group(1)
body = template[template.index("## Commands"):]
assert "{{display}}" not in body, "the template's body names the app: keep {{display}} to its first paragraph"
header = f"""# Ferrokit for AI coding agents

Read this before writing code with `ferrokit` {version}. Ferrokit builds native Mac apps in Rust: every view is a real AppKit view (`NSTableView`, `NSTextView`…), built once and updated in place by signals. macOS 15 or later; everything runs on the main thread.

- **A new app:** `cargo install cargo-ferrokit`, then `cargo ferrokit new <name>` (`--document` for a document app). It comes with this file as its `AGENTS.md`, a working example and self-checks.
- **An existing project:** `cargo add ferrokit`, `cargo install cargo-ferrokit`, then `cargo ferrokit agents` writes this file into it as `AGENTS.md`. The self-checks in a new app's `src/main.rs` are the pattern to copy.

"""
open(sys.argv[1], "w").write(header + body.replace("{{version}}", version))
PY
if [ "${1:-}" = "--check" ]; then
  if cmp -s "$out" crates/ferrokit/AGENTS.md; then
    rm -f "$out"
    echo "AGENTS.md is current"
  else
    diff -u crates/ferrokit/AGENTS.md "$out" | head -20
    rm -f "$out"
    echo "crates/ferrokit/AGENTS.md is out of date: run scripts/agents-md.sh"
    exit 1
  fi
fi
