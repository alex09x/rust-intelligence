# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Checks by content every differing row of a `position_ab.py` report between two RIDX binaries
that differ by the body-end scanner of call hierarchy (a lifetime or label tick no longer opens a
char literal), and writes the classes sidecar next to the published copy.

    position_ab_body_end.py --repo DIR --report position_ab_<a>_vs_<b>_<node>_<corpus>.json \\
        --before BIN --after BIN --out position_ab_<a>_vs_<b>_<node>_<corpus>.classes.json

A row is noise when the two before processes disagree; it is recorded with the noise flag and no
class. Every other differing row is asked again from both binaries, and a model of the scanner in
both versions (`body_end` in the rules module) gives the old and the corrected body end of each
function involved:
  f1  incomingCalls: for every caller whose call sites changed, the call sites gained lie in the
      span the corrected scan newly covers (old end, new end] and the ones lost in the span the
      old scan wrongly covered (new end, old end]: the old scan ended at the first semicolon when
      a tick swallowed the opening brace, or ran into the next item when a tick swallowed the
      terminating semicolon or a closing brace.
  f2  outgoingCalls on a function: its callees gained and lost follow the same move of its own
      body end.
A row of any other method, or failing its check, is `unclassified` and the exit code is 1.
"""

import argparse
import json
import os
import sys
from collections import Counter
from pathlib import Path

bench_dir = Path(__file__).resolve().parent
if str(bench_dir) not in sys.path:
    sys.path.insert(0, str(bench_dir))

from lsp_process import SimpleLspProcess  # noqa: E402
from matrix_categories import build_params  # noqa: E402
from position_ab_body_end_rules import CLASS_INCOMING, CLASS_OUTGOING, classify_incoming, classify_outgoing  # noqa: E402
from position_ab_classify import scenario_from_record  # noqa: E402
from position_ab_content import differing_rows  # noqa: E402
from position_ab_content_rules import RPC, scrub_path  # noqa: E402


class Checker:
    def __init__(self, repo, before_bin, after_bin):
        self.repo = str(Path(repo).resolve())
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

    def check_row(self, row):
        key, rel = row["method"], row["file"]
        rec = {"name": row["name"], "file": rel, "line": row["line"], "character": row["character"], "symbol": row["symbol"]}
        params = build_params(key, scenario_from_record(key, rec, self.repo), self.repo)
        res_b, _, err_b = self.before.request(RPC[key], params, timeout=60.0)
        res_a, _, err_a = self.after.request(RPC[key], params, timeout=60.0)
        if key == "incoming_calls":
            ok, ev = classify_incoming(res_b, res_a, self.lines)
            cls = CLASS_INCOMING
        elif key == "outgoing_calls":
            ok, ev = classify_outgoing(res_b, res_a, self.lines(os.path.join(self.repo, rel)), row["line"])
            cls = CLASS_OUTGOING
        else:
            ok, ev, cls = False, {"reason": "method not covered by the body-end rules"}, "unclassified"
        if err_b or err_a:
            ev["error_before"], ev["error_after"] = err_b, err_a
        return (cls if ok else "unclassified"), self.scrub(ev)

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
