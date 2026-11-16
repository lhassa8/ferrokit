"""After mdBook: gives each page of the website what search engines and AI
assistants read, from the page's own Markdown.

    python3 -I scripts/site_post.py <out dir> <site url> <version>

For every page:
- a Markdown copy next to it (`compare.html` -> `compare.md`), with
  `{{#include}}`s expanded, linked from the page as `rel="alternate"`;
- its own description: a `<!-- description: ... -->` line in the source, else
  its first paragraph;
- a canonical URL, Open Graph and Twitter tags, and JSON-LD (the software on
  the home page, an article elsewhere, questions and answers on the FAQ);
- a visible line under the title: when the page last changed (git) and the
  version it describes.
Then `sitemap.xml` with each page's date.

Run from the repository's root (scripts/site.sh does).
"""

import html
import json
import pathlib
import re
import subprocess
import sys

out = pathlib.Path(sys.argv[1])
site = sys.argv[2].rstrip("/")
version = sys.argv[3]
minor = ".".join(version.split(".")[:2])
src = pathlib.Path("site/src")
image = "https://raw.githubusercontent.com/lhassa8/ferrokit/main/assets/social-preview.png"
skip = {"404.html", "print.html", "toc.html"}


def source_of(rel: str) -> pathlib.Path:
    return src / (rel[: -len(".html")] + ".md")


def origin_of(md: pathlib.Path) -> pathlib.Path:
    """The file git knows: the guide is copied in from the crate."""
    if md.parent.name == "guide":
        return pathlib.Path("crates/ferrokit/guide") / md.name
    return md


def changed(md: pathlib.Path) -> str:
    day = subprocess.run(["git", "log", "-1", "--format=%cs", "--", str(origin_of(md))], capture_output=True, text=True).stdout.strip()
    if not day:  # new and not committed yet
        day = subprocess.run(["date", "+%Y-%m-%d"], capture_output=True, text=True).stdout.strip()
    return day


def expand(md: pathlib.Path) -> str:
    text = md.read_text()

    def include(m):
        return (md.parent / m.group(1).strip()).read_text().rstrip("\n")

    return re.sub(r"\{\{#include ([^}]+)\}\}", include, text)


def plain(s: str) -> str:
    s = re.sub(r"!\[[^\]]*\]\([^)]*\)", "", s)
    s = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", s)
    s = s.replace("`", "").replace("**", "").replace("*", "")
    return " ".join(s.split())


def description(md_text: str) -> str:
    m = re.search(r"<!--\s*description:\s*(.*?)\s*-->", md_text, re.S)
    if m:
        return " ".join(m.group(1).split())
    for para in re.split(r"\n\s*\n", md_text):
        p = para.strip()
        if not p or p.startswith(("#", "```", "<", "|", "-", ">", "!")):
            continue
        d = plain(p)
        if len(d) > 158:
            d = d[:158].rsplit(" ", 1)[0].rstrip(",;:") + "…"
        return d
    return ""


def faq(md_text: str):
    """`## Question?` headings and the paragraphs under them."""
    items = []
    for m in re.finditer(r"^## ([^\n]+\?)[ \t]*\n(.*?)(?=^## |\Z)", md_text, re.M | re.S):
        answer = plain(re.sub(r"```[\w,]*\n(.*?)```", r"\1", m.group(2), flags=re.S))
        if answer:
            items.append({"@type": "Question", "name": plain(m.group(1)), "acceptedAnswer": {"@type": "Answer", "text": answer}})
    return items


software = {
    "@type": "SoftwareSourceCode",
    "name": "Ferrokit",
    "alternateName": "ferrokit (Rust crate)",
    "description": "A native macOS GUI framework for Rust: every view a real AppKit control (NSTableView, NSOutlineView, NSTextView), updated in place by fine-grained signals.",
    "codeRepository": "https://github.com/lhassa8/ferrokit",
    "url": site + "/",
    "programmingLanguage": "Rust",
    "runtimePlatform": "macOS 15 or later",
    "license": "MIT OR Apache-2.0",
    "version": version,
    "sameAs": ["https://crates.io/crates/ferrokit", "https://docs.rs/ferrokit", "https://lib.rs/crates/ferrokit"],
}

sitemap = []
for page in sorted(out.rglob("*.html")):
    rel = page.relative_to(out).as_posix()
    if rel in skip:
        continue
    md = source_of(rel)
    if not md.exists():
        continue
    md_text = expand(md)
    (out / (rel[: -len(".html")] + ".md")).write_text(md_text)
    day = changed(md)
    url = f"{site}/{rel}" if rel != "index.html" else f"{site}/"
    md_url = f"{site}/{rel[: -len('.html')]}.md"
    h1 = re.search(r"^# (.+)$", md_text, re.M)
    headline = plain(h1.group(1)) if h1 else "Ferrokit"
    desc = description(md_text) or software["description"]

    if rel == "index.html":
        data = dict(software, **{"@context": "https://schema.org", "dateModified": day})
    elif rel == "faq.html":
        data = {"@context": "https://schema.org", "@type": "FAQPage", "dateModified": day, "about": {"@type": "SoftwareSourceCode", "name": "Ferrokit", "codeRepository": software["codeRepository"]}, "mainEntity": faq(md_text)}
    else:
        data = {
            "@context": "https://schema.org",
            "@type": "TechArticle",
            "headline": headline,
            "description": desc,
            "dateModified": day,
            "url": url,
            "proficiencyLevel": "Beginner" if rel.startswith("guide/01") else "Intermediate",
            "about": {"@type": "SoftwareSourceCode", "name": "Ferrokit", "codeRepository": software["codeRepository"], "version": version},
            "author": {"@type": "Person", "name": "Lars Tray", "url": "https://github.com/lhassa8"},
        }

    e = html.escape
    head = "\n".join(
        [
            f'        <link rel="canonical" href="{e(url)}">',
            f'        <link rel="alternate" type="text/markdown" href="{e(md_url)}">',
            '        <meta property="og:type" content="website">',
            '        <meta property="og:site_name" content="Ferrokit">',
            f'        <meta property="og:title" content="{e(headline)}">',
            f'        <meta property="og:description" content="{e(desc)}">',
            f'        <meta property="og:url" content="{e(url)}">',
            f'        <meta property="og:image" content="{image}">',
            '        <meta name="twitter:card" content="summary_large_image">',
            f'        <script type="application/ld+json">{json.dumps(data, ensure_ascii=False)}</script>',
        ]
    )
    text = page.read_text()
    text, n = re.subn(r'<meta name="description" content="[^"]*">', lambda _: f'<meta name="description" content="{e(desc)}">\n' + head, text, count=1)
    assert n == 1, f"{rel}: no description tag"
    # The date and version under the page's title (not on the home page).
    if rel != "index.html":
        line = f'<p class="page-meta"><small>Updated {day} · Ferrokit {minor} · <a href="{e(md_url)}">Markdown</a></small></p>'
        text = re.sub(r"(<main>.*?<h1[^>]*>.*?</h1>)", lambda m: m.group(1) + "\n" + line, text, count=1, flags=re.S)
    page.write_text(text)
    sitemap.append((url, day))

with open(out / "sitemap.xml", "w") as f:
    f.write('<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n')
    for url, day in sitemap:
        f.write(f"  <url><loc>{url}</loc><lastmod>{day}</lastmod></url>\n")
    f.write("</urlset>\n")
print(f"site: {len(sitemap)} pages with descriptions, Markdown copies and structured data")
