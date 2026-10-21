# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Checks by content every differing row of a `position_ab.py` report between two RIDX binaries
that differ by the cursor precedence rule (a definition at the cursor answers instead of a
reference whose span encloses it), and writes the classes sidecar next to the published copy.

    position_ab_cursor_def.py --repo DIR --report position_ab_<a>_vs_<b>_<node>_<corpus>.json \\
        --before BIN --after BIN --census <corpus>.ws.tsv [--census <corpus>.deps.tsv] \\
        [--previous <earlier>.classes.json] --out position_ab_<a>_vs_<b>_<node>_<corpus>.classes.json

`--census` files are `ref_spans` outputs (the example in crates/ridx-format/examples) with the
`self` column; the kept rows wide enough to enclose a definition are the references the rule looks
behind. A row is noise when the two before processes disagree; it is recorded with the noise flag
and no class. Every other differing row is asked again from both binaries for the removed and
added locations, and the reader's definition at the cursor is asked from both binaries
(`ridx def <file>:<line>:<col> --json`):
  d1  the cursor lies inside such a reference; the before definition was not at the cursor (the
      reference's target, reached through the enclosing reference) and the after definition is an
      item whose span contains the cursor and lies inside the reference's span. The evidence says
      whether the file is a test, bench or example target and whether the item sits in a `tests`
      module, and carries the class the row had in `--previous` (the A/B of the name-reference
      rule: s6 rows are the ones this rule was written for).
A row failing its check is `unclassified`. Paths in the evidence are relative to the workspace,
the cargo registry, a git checkout or the sysroot source tree; no private prefix is kept.
"""

import argparse
import json
import os
import subprocess
import sys
from collections import Counter
from pathlib import Path

bench_dir = Path(__file__).resolve().parent
if str(bench_dir) not in sys.path:
    sys.path.insert(0, str(bench_dir))

from lsp_process import SimpleLspProcess  # noqa: E402
from matrix_categories import build_params  # noqa: E402
from position_ab_classify import scenario_from_record  # noqa: E402
from position_ab_content import differing_rows  # noqa: E402
from position_ab_content_rules import RPC, scrub_path  # noqa: E402
from position_ab_cursor_def_rules import (  # noqa: E402
    CLASS_ITEM, classify_cursor_definition, in_test_file, in_test_module, load_enclosing_refs, one_edit_form,
)
from position_ab_name_span_rules import locations_of, utf16_to_char  # noqa: E402


def loc_key(loc):
    return (loc[0] or "", loc[1])


def previous_classes(path):
    """(method, file, line, character) -> class of an earlier classes sidecar."""
    if not path:
        return {}
    prev = json.load(open(path, encoding="utf-8"))
    return {(r["method"], r["file"], r["line"], r["character"]): r["class"] for r in prev["rows"]}


class Checker:
    def __init__(self, repo, before_bin, after_bin, enclosing):
        self.repo = str(Path(repo).resolve())
        self.enclosing = enclosing
        self.before_bin, self.after_bin = before_bin, after_bin
        self.before = SimpleLspProcess([before_bin, "lsp"], self.repo, is_ra=False)
        self.after = SimpleLspProcess([after_bin, "lsp"], self.repo, is_ra=False)
        self.lines_cache = {}

    def close(self):
        self.before.close()
        self.after.close()

    def lines(self, abs_path):
        if abs_path not in self.lines_cache:
            try:
                self.lines_cache[abs_path] = Path(abs_path).read_text(encoding="utf-8", errors="replace").split("\n")
            except OSError:
                self.lines_cache[abs_path] = None
        return self.lines_cache[abs_path]

    def cli_def(self, bin_path, rel, line1, col1):
        try:
            out = subprocess.run([bin_path, "def", f"{rel}:{line1}:{col1}", "--json"], cwd=self.repo, capture_output=True, text=True, timeout=60)
            return json.loads(out.stdout)
        except (OSError, ValueError, subprocess.TimeoutExpired):
            return None

    def check_row(self, row):
        key, rel = row["method"], row["file"]
        abs_path = os.path.join(self.repo, rel)
        rec = {"name": row["name"], "file": rel, "line": row["line"], "character": row["character"], "symbol": row["symbol"]}
        params = build_params(key, scenario_from_record(key, rec, self.repo), self.repo)
        res_b, _, err_b = self.before.request(RPC[key], params, timeout=60.0)
        res_a, _, err_a = self.after.request(RPC[key], params, timeout=60.0)
        lines = self.lines(abs_path)
        # Row positions are 1-based (line, UTF-16 column); the request builder subtracts one.
        line1, utf16_col0 = row["line"], row["character"] - 1
        col1 = utf16_to_char(lines[line1 - 1], utf16_col0) + 1 if lines and 0 < line1 <= len(lines) else utf16_col0 + 1
        cb = Counter(locations_of(one_edit_form(res_b), self.lines))
        ca = Counter(locations_of(one_edit_form(res_a), self.lines))
        removed = sorted((cb - ca).elements(), key=loc_key)
        added = sorted((ca - cb).elements(), key=loc_key)
        db = self.cli_def(self.before_bin, rel, line1, col1)
        da = self.cli_def(self.after_bin, rel, line1, col1)
        ok, ev = classify_cursor_definition((line1, col1), db, da, self.enclosing.get(abs_path, []))
        ev.update({
            "removed_count": len(removed), "added_count": len(added),
            "removed": [[p, list(s)] for p, s in removed[:8]], "added": [[p, list(s)] for p, s in added[:8]],
            "in_test_file": in_test_file(rel), "in_test_module": in_test_module((da or {}).get("path")),
        })
        if err_b or err_a:
            ev["error_before"], ev["error_after"] = err_b, err_a
        return (CLASS_ITEM if ok else "unclassified"), self.scrub(ev)

    def scrub(self, evidence):
        def walk(x):
            if isinstance(x, list) and len(x) == 2 and isinstance(x[0], str) and isinstance(x[1], list):
                return [scrub_path(x[0], self.repo), x[1]]
            if isinstance(x, list):
                return [walk(v) for v in x]
            if isinstance(x, dict):
                return {k: walk(v) for k, v in x.items()}
            return x

        return walk(evidence)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--repo", required=True)
    ap.add_argument("--report", required=True, help="full position_ab report (all rows)")
    ap.add_argument("--before", required=True, help="ridx binary of the first server")
    ap.add_argument("--after", required=True, help="ridx binary of the second server")
    ap.add_argument("--census", action="append", required=True, help="ref_spans output with the self column; repeatable")
    ap.add_argument("--previous", help="classes sidecar of an earlier A/B on the same positions")
    ap.add_argument("--out", required=True)
    args = ap.parse_args(argv)
    rep = json.load(open(args.report, encoding="utf-8"))
    names = [s["name"] for s in rep["servers"]]
    repo = str(Path(args.repo).resolve())
    enclosing = load_enclosing_refs(args.census, repo)
    previous = previous_classes(args.previous)
    checker = Checker(repo, args.before, args.after, enclosing)
    rows, summary, by_method = [], Counter(), Counter()
    try:
        for row, noise in differing_rows(rep, names):
            entry = {k: row[k] for k in ("method", "file", "line", "character", "symbol", "target_kind")}
            entry["noise_before_vs_before2"] = noise
            if noise:
                entry["class"], entry["evidence"] = None, {}
                summary["noise"] += 1
            else:
                entry["class"], entry["evidence"] = checker.check_row(row)
                entry["evidence"]["previous_class"] = previous.get((row["method"], row["file"], row["line"], row["character"]))
                summary[entry["class"]] += 1
                by_method[row["method"]] += 1
            rows.append(entry)
    finally:
        checker.close()
    out = {
        "schema": "position_ab_classes/1", "repo_dir_name": rep["repo_dir_name"], "report": os.path.basename(args.report),
        "servers": [{k: s[k] for k in ("name", "commit", "dirty", "binary") if k in s} for s in rep["servers"]],
        "census": [os.path.basename(p) for p in args.census], "previous": os.path.basename(args.previous) if args.previous else None,
        "enclosing_references": sum(len(v) for v in enclosing.values()),
        "rows_differing": len(rows), "summary": dict(sorted(summary.items())), "rows_by_method": dict(sorted(by_method.items())),
        "rows": rows,
    }
    Path(args.out).write_text(json.dumps(out, indent=1, sort_keys=False) + "\n", encoding="utf-8")
    print(f"{rep['repo_dir_name']}: differing {len(rows)}, classes {dict(sorted(summary.items()))}; wrote {args.out}")
    return 1 if summary["unclassified"] else 0


if __name__ == "__main__":
    sys.exit(main())
