#!/bin/zsh
# Builds the Ferrokit website into target/site (mdBook, `cargo install mdbook`).
#
#   scripts/site.sh            compile the code samples, then build
#   scripts/site.sh --serve    the same, then serve it at http://localhost:3000/
#
# The guide's chapters are copied in from crates/ferrokit/guide (their links
# to docs.rs's copy of the guide are pointed at the site's pages), the
# screenshots from assets/, and llms.txt, llms-full.txt and sitemap.xml are
# written from the crate's own AGENTS.md and API.md, so none of it drifts.
set -euo pipefail
root=${0:A:h:h}
cd "$root"
site=https://lhassa8.github.io/ferrokit

command -v mdbook >/dev/null || { echo "site.sh: needs mdbook (cargo install mdbook --locked)" >&2; exit 1; }

echo "site: compiling the code samples"
CARGO_TARGET_DIR=target cargo clippy -q --manifest-path site/snippets/Cargo.toml --bins -- -D warnings

echo "site: copying the guide"
rm -rf site/src/guide site/src/images
mkdir -p site/src/guide site/src/images
for f in crates/ferrokit/guide/[0-9][0-9]-*.md; do
  # docs.rs guide links -> the site's pages; intra-doc links -> docs.rs search
  perl -pe '
    s{https://docs\.rs/ferrokit/latest/ferrokit/guide/ch(\d\d)_([a-z_]+)/}{my ($n,$s)=($1,$2); $s=~tr/_/-/; "$n-$s.md"}ge;
    s{\]\(crate::(?:[A-Za-z_]+::)*([A-Za-z_]+)\)}{](https://docs.rs/ferrokit/latest/ferrokit/?search=$1)}g;
  ' "$f" > "site/src/guide/${f:t}"
done
cp assets/sql-browser.png assets/habits.png assets/charts-dark.png site/src/images/

echo "site: building"
mdbook build site

out=target/site
echo "site: writing llms.txt, llms-full.txt, sitemap.xml"
{
  print -r -- "# Ferrokit: native macOS GUI framework for Rust"
  print
  print -r -- "> Ferrokit builds native Mac apps in Rust: every view is a real AppKit control (NSTableView, NSOutlineView, NSTextView, NSSplitView), built once and updated in place by fine-grained signals. VoiceOver, spell check, input methods and Mac text editing come from macOS itself. Mac only (macOS 15 or later); version $(grep -m1 '^version' Cargo.toml | cut -d'"' -f2), an alpha whose API still changes. Crate: ferrokit; tool: cargo-ferrokit (new, run, preview, check, bundle, notarize, appcast, bundle --app-store)."
  print
  print -r -- "Use it for Mac-only apps that should look and behave like Mac apps, especially data-heavy ones (tables, outlines, text editors, logs, charts). Don't use it for apps that must also run on Windows, Linux, mobile or the web (egui, Iced, Slint, Tauri or NSH fit those), for games, or for a UI inside a GPU-rendered window."
  print
  print -r -- "## Docs for agents"
  print
  print -r -- "- [AGENTS.md]($site/AGENTS.md): read first. Rules, recipes that compile, known limits and workarounds."
  print -r -- "- [API.md]($site/API.md): every public item with its signature and first doc paragraph."
  print -r -- "- [llms-full.txt]($site/llms-full.txt): AGENTS.md and API.md in one file."
  print
  print -r -- "## How to"
  print
  for p in native-mac-app-in-rust nstableview-from-rust voiceover-and-mac-text mac-app-store wrap-any-nsview compare; do
    title=$(grep -m1 '^# ' site/src/$p.md | sed 's/^# //')
    print -r -- "- [$title]($site/$p.html)"
  done
  print
  print -r -- "## Guide"
  print
  for f in site/src/guide/[0-9][0-9]-*.md; do
    title=$(grep -m1 '^# ' $f | sed 's/^# //')
    print -r -- "- [$title]($site/guide/${f:t:r}.html)"
  done
  print
  print -r -- "## Optional"
  print
  print -r -- "- [API reference on docs.rs](https://docs.rs/ferrokit)"
  print -r -- "- [Source and example apps](https://github.com/lhassa8/ferrokit)"
} > $out/llms.txt
cp crates/ferrokit/AGENTS.md crates/ferrokit/API.md $out/
{ cat crates/ferrokit/AGENTS.md; print; print -r -- "---"; print; cat crates/ferrokit/API.md } > $out/llms-full.txt
{
  print -r -- '<?xml version="1.0" encoding="UTF-8"?>'
  print -r -- '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">'
  for f in $out/**/*.html(N); do
    rel=${f#$out/}
    [[ $rel == 404.html || $rel == print.html || $rel == toc.html ]] && continue
    print -r -- "  <url><loc>$site/$rel</loc></url>"
  done
  print -r -- '</urlset>'
} > $out/sitemap.xml

echo "site: done, $out/index.html"
if [[ ${1:-} == --serve ]]; then
  exec python3 -m http.server 3000 --directory target/site --bind 127.0.0.1
fi
