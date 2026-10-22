# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Checks by content every differing row of a `position_ab.py` report between two RIDX binaries
that differ by the enclosing-function lookup of call hierarchy (`find_enclosing_function` tries
every shard indexing the call site's file instead of returning after the first), and writes the
classes sidecar next to the published copy.

    position_ab_enclosing.py --repo DIR --report position_ab_<a>_vs_<b>_<node>_<corpus>.json \\
        --before BIN --after BIN --out position_ab_<a>_vs_<b>_<node>_<corpus>.classes.json

A row is noise when the two before processes disagree; it is recorded with the noise flag and no
class. Every other differing row is asked again from both binaries. Only incomingCalls rows are
expected: no caller item and no call site may be repeated (a caller held by two shards appears
once), every caller of the before answer must be in the after answer unchanged, and every caller
gained must be the reader's definition at its name (`ridx def`, after binary, same line), have its
call sites inside its body (the body-end model of the rules module of
the scanner A/B, lifetime-aware) with the item range ending there, and be
  e1  a test-only function of its file: the definition path names a module that opens in the
      file under a `#[cfg(... test ...)]` attribute, or the function header itself stands under a
      `#[cfg(... test ...)]` or `#[test]` attribute; the lib target's shard of the crate, which sorts first,
      indexes the file without these functions, the test target's shard holds them;
  e2  a function of a file under `tests/`, `examples/` or `benches/` of a package: the file is
      indexed by that target's shard, another shard of the package sorts first.
A row of any other method, or failing its check, is `unclassified` and the exit code is 1.
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
from position_ab_enclosing_rules import classify_added_callers  # noqa: E402


class Checker:
    def __init__(self, repo, before_bin, after_bin):
        self.repo = str(Path(repo).resolve())
        self.after_bin = after_bin
        self.before = SimpleLspProcess([before_bin, "lsp"], self.repo, is_ra=False)
        self.after = SimpleLspProcess([after_bin, "lsp"], self.repo, is_ra=False)
        self.lines_cache = {}

    def close(self):
        self.before.close()
        self.after.close()

    def lines(self, abs_path):
        if abs_path not in self.lines_cache:
            try:
                text = Path(abs_path).read_text(encoding="utf-8", errors="replace")
                # like Rust's `str::lines`: no empty line after a final newline
                self.lines_cache[abs_path] = text.split("\n")[:-1] if text.endswith("\n") else text.split("\n")
            except OSError:
                self.lines_cache[abs_path] = None
        return self.lines_cache[abs_path]

    def def_at(self, abs_path, line, name):
        """(definition path, 1-based line) of the reader's definition at the caller's name
        (`ridx def <file>:<line>:<col> --json`, after binary, 1-based char column of the name in the
        header line), or (None, None)."""
        lines = self.lines(abs_path) or []
        if not 0 < line <= len(lines):
            return None, None
        col = lines[line - 1].find(name)
        if col < 0:
            return None, None
        rel = os.path.relpath(abs_path, self.repo)
        try:
            out = subprocess.run([self.after_bin, "def", f"{rel}:{line}:{col + 1}", "--json"], cwd=self.repo, capture_output=True, text=True, timeout=60)
            d = json.loads(out.stdout)
        except (OSError, ValueError, subprocess.TimeoutExpired):
            return None, None
        if not isinstance(d, dict):
            return None, None
        return d.get("path"), d.get("line", d.get("start_line"))

    def check_row(self, row):
        key, rel = row["method"], row["file"]
        rec = {"name": row["name"], "file": rel, "line": row["line"], "character": row["character"], "symbol": row["symbol"]}
        params = build_params(key, scenario_from_record(key, rec, self.repo), self.repo)
        res_b, _, err_b = self.before.request(RPC[key], params, timeout=60.0)
        res_a, _, err_a = self.after.request(RPC[key], params, timeout=60.0)
        if key == "incoming_calls":
            cls, ev = classify_added_callers(res_b, res_a, self.lines, self.def_at, self.repo)
        else:
            cls, ev = None, {"reason": "method not covered by the enclosing-function rules"}
        if err_b or err_a:
            cls, ev["error_before"], ev["error_after"] = None, err_b, err_a
        return (cls or "unclassified"), self.scrub(ev)

    def scrub(self, evidence):
        def walk(x):
            if isinstance(x, str) and x.startswith("/"):
                return scrub_path(x, self.repo)
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
    ap.add_argument("--before", required=True)
    ap.add_argument("--after", required=True)
    ap.add_argument("--out", required=True)
    args = ap.parse_args(argv)
    rep = json.load(open(args.report, encoding="utf-8"))
    names = [s["name"] for s in rep["servers"]]
    checker = Checker(args.repo, args.before, args.after)
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
                summary[entry["class"]] += 1
                by_method[row["method"]] += 1
            rows.append(entry)
    finally:
        checker.close()
    out = {
        "schema": "position_ab_classes/1", "repo_dir_name": rep["repo_dir_name"], "report": os.path.basename(args.report),
        "servers": [{k: s[k] for k in ("name", "commit", "dirty", "binary") if k in s} for s in rep["servers"]],
        "rows_differing": len(rows), "summary": dict(sorted(summary.items())), "rows_by_method": dict(sorted(by_method.items())),
        "rows": rows,
    }
    Path(args.out).write_text(json.dumps(out, indent=1, sort_keys=False) + "\n", encoding="utf-8")
    print(f"{rep['repo_dir_name']}: differing {len(rows)}, classes {dict(sorted(summary.items()))}; wrote {args.out}")
    return 1 if summary["unclassified"] else 0


if __name__ == "__main__":
    sys.exit(main())
