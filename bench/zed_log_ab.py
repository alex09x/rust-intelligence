# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""A/B of two ridx binaries on the position requests an editor really sent.

    zed_log_ab.py --root DIR --logs 'GLOB' --a OLD_BIN --b NEW_BIN [--limit 400] [--out FILE]

Reads the RPC logs `ridx-lsp-logged` writes (`>> req textDocument/definition id=… {params}`), keeps the position
requests on files of `--root` the editor never edited in that log (their text is the one on disk), and asks both
binaries the same requests in an editor-like session (each file opened with its text from disk). Prints how many
answers are the same, gained (null → answer), lost (answer → null) or changed, with the rows that differ.
"""

import argparse
import glob
import json
import os
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from lsp_process import SimpleLspProcess  # noqa: E402

METHODS = ("definition", "hover", "references", "typeDefinition", "implementation", "documentHighlight", "declaration")
REQ = re.compile(r">> req textDocument/(\w+) id=\S+ (\{.*\})\s*$")
EDIT = re.compile(r">> notif textDocument/didChange id=\S+ \{\"textDocument\": \{\"uri\": \"([^\"]+)\"")


def positions(logs, root, limit):
    edited, seen = set(), {}
    for log in logs:
        for line in open(log, errors="replace"):
            if m := EDIT.search(line):
                edited.add(m.group(1))
                continue
            m = REQ.search(line)
            if not m or m.group(1) not in METHODS:
                continue
            try:
                p = json.loads(m.group(2))
            except json.JSONDecodeError:
                continue
            uri = p.get("textDocument", {}).get("uri", "")
            pos = p.get("position")
            if not uri.startswith(Path(root).as_uri()) or not pos:
                continue
            seen[(m.group(1), uri, pos["line"], pos["character"])] = p
    rows = [k for k in seen if k[1] not in edited and Path(k[1].removeprefix("file://")).is_file()]
    rows.sort()
    step = max(1, len(rows) // limit) if limit else 1
    return rows[::step][:limit] if limit else rows


def sweep(root, files, per_file, methods):
    """Positions an editor sends without a log: each name's first character and the caret just past it, in the first
    `files` sources of `root` (outside `target/`), at most `per_file` names a file."""
    srcs = sorted(p for p in Path(root).rglob("*.rs") if "target" not in p.relative_to(root).parts)[:files]
    rows = []
    for p in srcs:
        lines = p.read_text(errors="replace").split("\n")
        names = [(i, m.start(), m.end()) for i, l in enumerate(lines) if not l.lstrip().startswith("//") for m in re.finditer(r"\b[A-Za-z_][A-Za-z0-9_]*\b", l)]
        step = max(1, len(names) // per_file)
        for i, s, e in names[::step][:per_file]:
            utf16 = lambda k: len(lines[i][:k].encode("utf-16-le")) // 2  # noqa: E731
            for m in methods:
                rows += [(m, p.as_uri(), i, utf16(s)), (m, p.as_uri(), i, utf16(e))]
    return rows


def ask(binary, root, rows):
    lsp = SimpleLspProcess([binary, "lsp"], cwd=root)
    out = {}
    try:
        for uri in sorted({r[1] for r in rows}):
            text = Path(uri.removeprefix("file://")).read_text(errors="replace")
            lsp.notify("textDocument/didOpen", {"textDocument": {"uri": uri, "languageId": "rust", "version": 0, "text": text}})
        for method, uri, line, ch in rows:
            params = {"textDocument": {"uri": uri}, "position": {"line": line, "character": ch}}
            if method == "references":
                params["context"] = {"includeDeclaration": True}
            res, _, err = lsp.request(f"textDocument/{method}", params, timeout=10)
            out[(method, uri, line, ch)] = {"error": str(err)} if err else res
    finally:
        lsp.close()
    return out


def norm(v):
    if isinstance(v, list):
        return sorted(json.dumps(x, sort_keys=True) for x in v)
    return json.dumps(v, sort_keys=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", required=True)
    ap.add_argument("--logs", default="")
    ap.add_argument("--sweep", type=int, default=0, help="also sweep the names of this many source files")
    ap.add_argument("--per-file", type=int, default=60)
    ap.add_argument("--methods", default="definition,hover,documentHighlight")
    ap.add_argument("--a", required=True)
    ap.add_argument("--b", required=True)
    ap.add_argument("--limit", type=int, default=400)
    ap.add_argument("--out", default="")
    a = ap.parse_args()
    root = os.path.realpath(a.root)
    rows = positions(sorted(glob.glob(os.path.expanduser(a.logs))), root, a.limit) if a.logs else []
    if a.sweep:
        rows += sweep(root, a.sweep, a.per_file, a.methods.split(","))
    os.environ["RIDX_REINDEX_ON_SAVE"] = "0"
    ra, rb = ask(a.a, root, rows), ask(a.b, root, rows)
    tally, diffs = {}, []
    for k in rows:
        x, y = ra.get(k), rb.get(k)
        kind = "same" if norm(x) == norm(y) else "gained" if not x and y else "lost" if x and not y else "changed"
        tally[(k[0], kind)] = tally.get((k[0], kind), 0) + 1
        if kind != "same":
            diffs.append({"method": k[0], "file": k[1].removeprefix("file://" + root + "/"), "line": k[2] + 1, "char": k[3], "kind": kind, "a": x, "b": y})
    for (m, kind), n in sorted(tally.items()):
        print(f"{m:18} {kind:8} {n}")
    if a.sweep:
        # A caret just past a name should answer as the name's first character does: the share of names where it does.
        for m in a.methods.split(","):
            pairs = [(rows[i], rows[i + 1]) for i in range(0, len(rows) - 1) if rows[i][0] == m and rows[i + 1][0] == m and rows[i][1:3] == rows[i + 1][1:3] and rows[i][3] < rows[i + 1][3]]
            agree = lambda r: sum(norm(r.get(s)) == norm(r.get(e)) and r.get(s) not in (None, []) for s, e in pairs)  # noqa: E731
            print(f"{m:18} past-the-name answers as on it: a {agree(ra)}/{len(pairs)}  b {agree(rb)}/{len(pairs)}")
    for d in diffs:
        print(f"{d['kind']:8} {d['method']:16} {d['file']}:{d['line']}:{d['char']}  a={json.dumps(d['a'])[:110]}  b={json.dumps(d['b'])[:110]}")
    if a.out:
        Path(a.out).write_text(json.dumps(diffs, indent=1))


if __name__ == "__main__":
    main()
