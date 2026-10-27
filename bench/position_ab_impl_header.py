# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Checks by content every differing implementation row of a `position_ab.py` report between two
RIDX binaries (see position_ab_impl_header_rules.py) and writes the classes sidecar:

    position_ab_impl_header.py --repo DIR --report position_ab_<a>_vs_<b>_<node>_<corpus>.json \\
        --before BIN --after BIN --out position_ab_<a>_vs_<b>_<node>_<corpus>.classes.json

A row is noise when the two before processes disagree. A row of another method, or failing its
check, is `unclassified` and the exit code is 1."""

import argparse
import json
import os
import sys
from collections import Counter
from pathlib import Path

bench_dir = Path(__file__).resolve().parent
if str(bench_dir) not in sys.path:
    sys.path.insert(0, str(bench_dir))

from matrix_categories import build_params  # noqa: E402
from position_ab_classify import scenario_from_record  # noqa: E402
from position_ab_content import differing_rows  # noqa: E402
from position_ab_content_rules import RPC  # noqa: E402
from position_ab_impl_header_rules import classify_impl_header  # noqa: E402
from position_ab_item_position import Checker, build_out  # noqa: E402
from position_ab_name_span_rules import locations_of  # noqa: E402


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    for name in ("--repo", "--report", "--before", "--after", "--out"):
        ap.add_argument(name, required=True)
    args = ap.parse_args(argv)
    rep = json.load(open(args.report, encoding="utf-8"))
    names = [s["name"] for s in rep["servers"]]
    checker = Checker(args.repo, args.before, args.after)
    rows = []
    try:
        for row, noise in differing_rows(rep, names):
            entry = {k: row[k] for k in ("method", "file", "line", "character", "symbol", "target_kind")}
            entry["noise_before_vs_before2"] = noise
            if noise:
                entry["class"], entry["evidence"] = None, {}
            elif row["method"] != "implementation":
                entry["class"], entry["evidence"] = "unclassified", {"reason": "method not covered by the impl header rule"}
            else:
                rec = {"name": row["name"], "file": row["file"], "line": row["line"], "character": row["character"], "symbol": row["symbol"]}
                params = build_params("implementation", scenario_from_record("implementation", rec, checker.repo), checker.repo)
                res_b, _, err_b = checker.before.request(RPC["implementation"], params, timeout=60.0)
                res_a, _, err_a = checker.after.request(RPC["implementation"], params, timeout=60.0)
                cls, ev = classify_impl_header(res_b, res_a, checker.lines, locations_of)
                if err_b or err_a:
                    cls, ev["error_before"], ev["error_after"] = None, err_b, err_a
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
