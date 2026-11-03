#!/usr/bin/env python3
"""Writes crates/ferrokit/API.md: every public item of `ferrokit` (and the
`ferrokit-core` types it re-exports) on one page, with its signature and the
first paragraph of its docs, for searching (people and AI assistants).

Reads the rustdoc JSON that scripts/api-md.sh makes. Signatures are taken
from the source, so they read as written.
"""
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DOC = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "target/apijson/doc"
OUT = Path(sys.argv[2]) if len(sys.argv) > 2 else ROOT / "crates/ferrokit/API.md"

crates = {name: json.load(open(DOC / f"{name}.json")) for name in ("ferrokit", "ferrokit_core")}
lines_cache = {}


def kind(item):
    return next(iter(item["inner"]))


def inner(item):
    return item["inner"][kind(item)]


def lookup(crate, path):
    """The item at `path` (segments after the crate name), following uses."""
    d = crates[crate]
    cur = d["index"][str(d["root"])]
    for seg in path:
        found = None
        for i in inner(cur)["items"]:
            it = d["index"].get(str(i))
            if it is None:
                continue
            if kind(it) == "use" and not inner(it)["is_glob"] and inner(it)["name"] == seg:
                return resolve_use(crate, it)[1] if seg == path[-1] else None
            if it.get("name") == seg:
                found = it
                break
        if found is None:
            return None
        cur = found
    return crate, cur


def resolve_use(crate, use):
    u = inner(use)
    d = crates[crate]
    target = d["index"].get(str(u["id"])) if u["id"] is not None else None
    if target is not None:
        return crate, (crate, target)
    segs = u["source"].split("::")
    if segs[0] == "ferrokit_core":
        return "ferrokit_core", lookup("ferrokit_core", segs[1:])
    if segs[0] in ("crate", "self"):
        segs = segs[1:]
    return crate, lookup(crate, segs)


def public(item):
    return item.get("visibility") == "public"


def members(crate, module):
    """The public items a module shows, uses resolved and globs expanded."""
    d = crates[crate]
    out = []
    for i in inner(module)["items"]:
        it = d["index"].get(str(i))
        if it is None or not public(it):
            continue
        if kind(it) == "use":
            _, res = resolve_use(crate, it)
            if res is None:
                continue
            if inner(it)["is_glob"]:
                out += members(*res)
            else:
                out.append((res[0], res[1], inner(it)["name"]))
        else:
            out.append((crate, it, it["name"]))
    return out


def source(item):
    sp = item.get("span")
    if not sp:
        return []
    path = ROOT / sp["filename"]
    if path not in lines_cache:
        lines_cache[path] = path.read_text().splitlines()
    return lines_cache[path][sp["begin"][0] - 1 : sp["end"][0]]


def signature(item):
    """The declaration as written, up to its body."""
    text = []
    for line in source(item):
        s = line.strip()
        if not text and (s.startswith("#[") or s.startswith("///") or s.startswith("//")):
            continue
        text.append(s)
        if s.endswith("{") or s.endswith(";") or s.endswith("}"):
            break
    sig = " ".join(text)
    sig = re.sub(r"\s*\{\s*\}?$", "", sig).rstrip(";").rstrip(",")
    sig = re.sub(r"\s+", " ", sig).replace("( ", "(").replace(" )", ")").replace(", )", ")")
    return sig


def summary(item):
    docs = (item.get("docs") or "").strip()
    if not docs:
        return ""
    first = docs.split("\n\n")[0]
    first = re.sub(r"\[([^\]]+)\]\([^)]*\)", r"\1", first)  # [text](link) -> text
    first = re.sub(r"\[(`[^`]+`)\]", r"\1", first)  # [`x`] -> `x`
    return re.sub(r"\s+", " ", first)


def methods(crate, item):
    """The public inherent methods (for a trait: its methods)."""
    d = crates[crate]
    k = kind(item)
    if k == "trait":
        ids = inner(item)["items"]
    else:
        ids = []
        for i in inner(item).get("impls", []):
            im = d["index"].get(str(i))
            if im is None:
                continue
            m = inner(im)
            if m["trait"] is None and not m["is_synthetic"] and m["blanket_impl"] is None:
                ids += m["items"]
    out = []
    for i in ids:
        it = d["index"].get(str(i))
        if it is None or kind(it) not in ("function", "assoc_const", "assoc_type"):
            continue
        if k != "trait" and not public(it):
            continue
        out.append(it)
    return out


def variants(crate, item):
    d = crates[crate]
    return [d["index"][str(v)] for v in inner(item).get("variants", []) if str(v) in d["index"]]


def fields(crate, item):
    d = crates[crate]
    kind_ = inner(item).get("kind", {})
    ids = kind_.get("plain", {}).get("fields", []) if isinstance(kind_, dict) else []
    return [d["index"][str(f)] for f in ids if str(f) in d["index"] and public(d["index"][str(f)])]


def entry_sig(crate, item):
    """The signature; a function made by a macro is shown by its name."""
    sig = signature(item)
    name = item.get("name") or ""
    if kind(item) == "function" and name and f"fn {name}" not in sig:
        params = [p[0] for p in inner(item)["sig"]["inputs"]]
        return f"fn {name}({', '.join(params)})"
    return sig or name


def entry(line, item):
    s = summary(item)
    return f"- `{line}`" + (f": {s}" if s else "")


def section_of(crate, item):
    """The defining module's file, for grouping."""
    sp = item.get("span") or {}
    return Path(sp.get("filename", "other")).stem


def render_type(out, crate, item, name):
    k = kind(item)
    out.append(f"### `{name}`" + (" (trait)" if k == "trait" else ""))
    out.append("")
    out.append(f"`{signature(item)}`" + (f": {summary(item)}" if summary(item) else ""))
    out.append("")
    if k == "enum":
        for v in variants(crate, item):
            out.append(entry(signature(v) or v["name"], v))
    for f in fields(crate, item):
        out.append(entry(signature(f), f))
    for m in methods(crate, item):
        out.append(entry(entry_sig(crate, m), m))
    out.append("")


def render_module(out, crate, module, title, depth, seen):
    items = members(crate, module)
    groups = {}
    order = []
    for c, it, name in items:
        key = (c, it["id"])
        if key in seen or kind(it) in ("module",):
            continue
        seen.add(key)
        sec = section_of(c, it)
        if sec not in groups:
            groups[sec] = []
            order.append(sec)
        groups[sec].append((c, it, name))
    if depth == 2:
        first = ["lib", "signal", "view", "layout", "appkit_views", "controls", "models", "table", "outline", "text", "commands", "chrome", "dialogs"]
        order.sort(key=lambda s: first.index(s) if s in first else len(first))
    for sec in order:
        if depth == 2:
            out.append(f"## {'App and windows (lib)' if sec == 'lib' else sec}")
        else:
            out.append(f"{'#' * depth} {sec}")
        out.append("")
        free = [(c, it, n) for c, it, n in groups[sec] if kind(it) in ("function", "constant", "static", "type_alias", "macro")]
        for c, it, n in free:
            out.append(entry(entry_sig(c, it), it))
        if free:
            out.append("")
        for c, it, n in groups[sec]:
            if kind(it) in ("struct", "enum", "trait", "union"):
                render_type(out, c, it, n)
    for c, it, name in items:
        if kind(it) == "module" and name != "guide":
            out.append(f"## Module `{name}`")
            out.append("")
            if summary(it):
                out.append(summary(it))
                out.append("")
            render_module(out, c, it, f"`{name}`", 3, seen)


def main():
    d = crates["ferrokit"]
    root = d["index"][str(d["root"])]
    version = d.get("crate_version") or ""
    out = [
        f"# ferrokit {version} API",
        "",
        "Every public item of `ferrokit`, and the `ferrokit-core` types it re-exports, on one page: the signature as written and the first paragraph of its docs. Generated by `scripts/api-md.sh` from rustdoc; the full docs, and the guide, are on [docs.rs](https://docs.rs/ferrokit). Items are grouped by the source file that defines them.",
        "",
        "Conventions: views are built once and update from signals; models (`TableModel`, `TextModel`) are created once; row indices are model indices; text offsets are UTF-8 bytes; everything runs on the main thread.",
        "",
    ]
    body = []
    render_module(body, "ferrokit", root, "", 2, set())
    heads = [l[3:] for l in body if l.startswith("## ")]
    anchor = lambda h: re.sub(r"[^a-z0-9 _-]", "", h.lower()).replace(" ", "-")
    out += [
        "**Where to look:** app, windows, menus and the shell → App and windows, `chrome`, `dialogs`, `commands`; state → `signal`, `resource`, `store`, `undo`; views and modifiers → `view`, `layout`, `appkit_views`, `controls`, `style`, `motion`; tables, outlines, text → `models`, `table`, `outline`, `text`; charts → `charts`, `canvas`; checks → Module `testing`.",
        "",
        "**Contents:** " + " · ".join(f"[{h.replace('`', '')}](#{anchor(h)})" for h in heads),
        "",
    ]
    out += body
    text = "\n".join(out).rstrip() + "\n"
    text = re.sub(r"\n{3,}", "\n\n", text)
    OUT.write_text(text)
    print(f"{OUT}: {text.count(chr(10))} lines")


main()
