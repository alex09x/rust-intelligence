# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Checks by content every differing row of a `position_ab.py` report between two RIDX binaries
that differ by what a use of an enum variant resolves to (classes in position_ab_variant_rules.py)
and writes the classes sidecar:

    position_ab_variant.py --repo DIR --report position_ab_<a>_vs_<b>_<node>_<corpus>.json \\
        --before BIN --after BIN --out position_ab_<a>_vs_<b>_<node>_<corpus>.classes.json \\
        [--shard K/N | --merge PART ...]

A row is noise when the two before processes disagree. A row of another method, or failing its
check, is `unclassified` and the exit code is 1."""

import argparse
import json
import os
import sys
from pathlib import Path

bench_dir = Path(__file__).resolve().parent
if str(bench_dir) not in sys.path:
    sys.path.insert(0, str(bench_dir))

from matrix_categories import build_params  # noqa: E402
from position_ab_classify import scenario_from_record  # noqa: E402
from position_ab_content import differing_rows  # noqa: E402
from position_ab_content_rules import RPC  # noqa: E402
from position_ab_item_position import Checker, build_out  # noqa: E402
from position_ab_name_span_rules import locations_of, utf16_to_char  # noqa: E402
from position_ab_variant_rules import (  # noqa: E402
    callee_before_paren, classify_definition, name_at, classify_hover, classify_signature, classify_variant_locations, cursor_token, declares_at)


def hover_text(res):
    contents = (res or {}).get("contents") if isinstance(res, dict) else None
    return contents.get("value") if isinstance(contents, dict) else (contents if isinstance(contents, str) else None)


def signature_label(res):
    sigs = (res or {}).get("signatures") if isinstance(res, dict) else None
    return sigs[0].get("label") if sigs else None


def check_row(checker, row):
    method = row["method"]
    rec = {k: row[k] for k in ("name", "file", "line", "character", "symbol")}
    abs_path = os.path.join(checker.repo, row["file"])
    lines = checker.lines(abs_path)
    token = cursor_token(lines, row["line"], row["character"])
    params = build_params(method, scenario_from_record(method, rec, checker.repo), checker.repo)
    res_b, _, err_b = checker.before.request(RPC[method], params, timeout=60.0)
    res_a, _, err_a = checker.after.request(RPC[method], params, timeout=60.0)
    if err_b or err_a:
        return None, {"error_before": err_b, "error_after": err_a}

    def definition_at(character, line=None):
        """Locations of textDocument/definition (after binary) at a 0-based character of a line."""
        position = {"line": (line or row["line"]) - 1, "character": character}
        res, _, err = checker.after.request(RPC["definition"], {"textDocument": {"uri": "file://" + abs_path}, "position": position}, timeout=60.0)
        return [] if err else locations_of(res, checker.lines)

    if method == "definition":
        return classify_definition(res_b, res_a, checker.lines, token, locations_of)
    if method == "hover":
        defs = definition_at(row["character"])
        return classify_hover(hover_text(res_b), hover_text(res_a), token, len(defs) == 1 and declares_at(checker.lines, defs[0], token or ""))
    if method == "signature_help":
        callee = callee_before_paren(lines, row["line"], utf16_to_char(lines[row["line"] - 1], row["character"]))
        defs = definition_at(callee[2], callee[1]) if callee else []
        return classify_signature(signature_label(res_b), signature_label(res_a) is None, callee[0] if callee else None,
                                  bool(callee) and len(defs) == 1 and declares_at(checker.lines, defs[0], callee[0]))
    if method in ("references", "document_highlight", "rename"):
        defs = definition_at(row["character"])
        cursor_def = None
        is_variant = False
        if len(defs) == 1:
            path, span = defs[0]
            cursor_def = (os.path.relpath(path, checker.repo) if os.path.isabs(path) else path, span[0])
            declared = name_at(checker.lines, defs[0])
            is_variant = declares_at(checker.lines, defs[0], declared or "")
        return classify_variant_locations(res_b, res_a, abs_path, checker.lines, locations_of, checker.def_at_col, cursor_def, is_variant)
    return None, {"reason": "method not covered by the variant rules"}


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    for name in ("--report", "--out"):
        ap.add_argument(name, required=True)
    for name in ("--repo", "--before", "--after", "--shard"):
        ap.add_argument(name)
    ap.add_argument("--merge", nargs="+")
    args = ap.parse_args(argv)
    rep = json.load(open(args.report, encoding="utf-8"))
    if args.merge:
        rows = [row for part in args.merge for row in json.load(open(part, encoding="utf-8"))["rows"]]
    else:
        todo = list(differing_rows(rep, [s["name"] for s in rep["servers"]]))
        if args.shard:
            k, n = map(int, args.shard.split("/"))
            size = -(-len(todo) // n)
            todo = todo[k * size:(k + 1) * size]
        checker = Checker(args.repo, args.before, args.after)
        rows = []
        try:
            for row, noise in todo:
                entry = {k: row[k] for k in ("method", "file", "line", "character", "symbol", "target_kind")}
                entry["noise_before_vs_before2"] = noise
                if noise:
                    entry["class"], entry["evidence"] = None, {}
                else:
                    cls, ev = check_row(checker, row)
                    entry["class"], entry["evidence"] = cls or "unclassified", checker.scrub(ev)
                rows.append(entry)
        finally:
            checker.close()
    out = build_out(rep, os.path.basename(args.report), rows)
    Path(args.out).write_text(json.dumps(out, indent=1) + "\n", encoding="utf-8")
    print(f"{rep['repo_dir_name']}: differing {len(rows)}, classes {out['summary']}; wrote {args.out}")
    return 1 if out["summary"].get("unclassified") else 0


if __name__ == "__main__":
    sys.exit(main())
