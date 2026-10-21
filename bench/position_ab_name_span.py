# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Checks by content every differing row of a `position_ab.py` report between two RIDX binaries
that differ by the name-reference rule (references whose span contains their own target's
definition are skipped), and writes the classes sidecar next to the published copy.

    position_ab_name_span.py --repo DIR --report position_ab_<a>_vs_<b>_<node>_<corpus>.json \\
        --before BIN --after BIN --census <corpus>.ws.tsv [--census <corpus>.deps.tsv] \\
        --out position_ab_<a>_vs_<b>_<node>_<corpus>.classes.json

`--census` files are `ref_spans` outputs (the example in crates/ridx-format/examples) with the
`self` column; the rows marked `contains` are the references the after binary skips. A row is noise
when the two before processes disagree; it is recorded with the noise flag and no class. Every
other differing row is asked again from both binaries and checked by content:
  s1  location lists (references, documentHighlight, rename, implementation, call hierarchy, ...):
      every removed location is a skipped reference and nothing was added; the evidence keeps the
      removed count and whether the two removed ranges nest (the whole function inside the whole
      file or module, as on a test function).
  s2  single locations (definition, declaration, typeDefinition): the cursor and the before answer
      lie inside one skipped reference of the cursor's file, and the after answer differs.
  s3  codeLens: the same lenses at the same positions, every changed title a reference count that
      fell by exactly the number of skipped references whose span contains the lens position.
  s4  any method, cursor inside a skipped reference (on the name of an item nested in a test
      function, or on no name): the reader's definition at the cursor (`ridx def`) used to resolve
      through that reference (it started inside the span) and now resolves to the item at the
      cursor, to the nameless crate-root module (the pre-existing fallback) or to nothing; the
      method's answer changed with it. A removed location that
      is the `#[test]` attribute (the generated constant's declaration) is accepted in s1.
  s5  outgoingCalls on a test function: the removed callee is the function itself, reached from a
      call site in its body through the enclosing reference (callee and call ranges lie inside a
      skipped reference); nothing was added.
  s6  cursor inside a `macro_rules!` invocation whose expansion references carry the invocation
      span: the before definition resolved through a skipped reference, the after definition is
      the target of a kept multi-line macro reference spanning the invocation. The item at the
      cursor is not reached either way; a definition at the cursor should win over an enclosing
      reference (follow-up).
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
from position_ab_name_span_rules import (  # noqa: E402
    CALL_KEYS, CLASS_FALLBACK, CLASS_INVOCATION, CLASS_LENS, CLASS_LIST, CLASS_SELF_CALL, CLASS_SINGLE, LENS_KEYS,
    SINGLE_KEYS, classify_fallback, classify_invocation, classify_lens, classify_list, classify_self_calls,
    classify_single, load_contains, load_invocation_refs, locations_of, utf16_to_char,
)


def loc_key(loc):
    """Sort key of a (path, span) location; bare ranges carry `None` as path."""
    return (loc[0] or "", loc[1])


class Checker:
    def __init__(self, repo, before_bin, after_bin, contains, invocation_refs):
        self.repo = str(Path(repo).resolve())
        self.contains = contains
        self.invocation_refs = invocation_refs
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
        """The reader-level definition at a 1-based position: `ridx def <file>:<line>:<col> --json`
        run in the workspace, the same lookup `find_references_at` starts from."""
        try:
            out = subprocess.run([bin_path, "def", f"{rel}:{line1}:{col1}", "--json"], cwd=self.repo, capture_output=True, text=True, timeout=60)
            return json.loads(out.stdout)
        except (OSError, ValueError, subprocess.TimeoutExpired):
            return None

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
        cursor = (line1, col1)
        in_file = self.contains.get(abs_path, set())
        if err_b or err_a:
            cls, ok, ev = "unclassified", False, {"error_before": err_b, "error_after": err_a}
        else:
            cls, ok, ev = self.check_answers(key, res_b, res_a, abs_path, cursor, in_file, lines)
        if not ok and key not in LENS_KEYS and any(classify_fallback(cursor, None, None, in_file)[1]["self_containing_spans_at_cursor"]):
            db = self.cli_def(self.before_bin, rel, line1, col1)
            da = self.cli_def(self.after_bin, rel, line1, col1)
            ok4, ev4 = classify_fallback(cursor, db, da, in_file)
            if ok4:
                ev4["removed_count"], ev4["added_count"] = ev.get("removed_count"), ev.get("added_count")
                if err_b or err_a:
                    ev4["error_before"], ev4["error_after"] = err_b, err_a
                return CLASS_FALLBACK, self.scrub(ev4)
            ok6, ev6 = classify_invocation(cursor, db, da, in_file, self.invocation_refs.get(abs_path, []))
            if ok6:
                ev6["removed_count"], ev6["added_count"] = ev.get("removed_count"), ev.get("added_count")
                if err_b or err_a:
                    ev6["error_before"], ev6["error_after"] = err_b, err_a
                return CLASS_INVOCATION, self.scrub(ev6)
            ev["fallback_check"] = ev4
        return (cls if ok else "unclassified"), self.scrub(ev)

    def check_answers(self, key, res_b, res_a, abs_path, cursor, in_file, lines):
        if key in LENS_KEYS:
            ok, ev = classify_lens(res_b, res_a, in_file, lines)
            cls = CLASS_LENS
        elif key in SINGLE_KEYS:
            lb = locations_of(res_b, self.lines)
            la = locations_of(res_a, self.lines)
            before = lb[0] if len(lb) == 1 else None
            after = la[0] if len(la) == 1 else None
            if len(lb) > 1 or len(la) > 1:
                removed = sorted(set(lb) - set(la), key=loc_key)
                added = sorted(set(la) - set(lb), key=loc_key)
                ok, ev = classify_list(removed, added, self.contains, abs_path, self.lines)
                cls = CLASS_LIST
            else:
                ok, ev = classify_single(cursor, before, after, in_file)
                cls = CLASS_SINGLE
        else:
            cb, ca = Counter(locations_of(res_b, self.lines)), Counter(locations_of(res_a, self.lines))
            removed = sorted((cb - ca).elements(), key=loc_key)
            added = sorted((ca - cb).elements(), key=loc_key)
            ok, ev = classify_list(removed, added, self.contains, abs_path, self.lines)
            cls = CLASS_LIST
            if not ok and key in CALL_KEYS:
                ok, ev = classify_self_calls(removed, added, in_file)
                cls = CLASS_SELF_CALL
        return cls, ok, ev


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--repo", required=True)
    ap.add_argument("--report", required=True, help="full position_ab report (all rows)")
    ap.add_argument("--before", required=True, help="ridx binary of the first server")
    ap.add_argument("--after", required=True, help="ridx binary of the second server")
    ap.add_argument("--census", action="append", required=True, help="ref_spans output with the self column; repeatable")
    ap.add_argument("--out", required=True)
    args = ap.parse_args(argv)
    rep = json.load(open(args.report, encoding="utf-8"))
    names = [s["name"] for s in rep["servers"]]
    repo = str(Path(args.repo).resolve())
    contains = load_contains(args.census, repo)
    checker = Checker(repo, args.before, args.after, contains, load_invocation_refs(args.census, repo))
    rows, summary, removed_counts = [], Counter(), {}
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
                if "removed_count" in entry["evidence"]:
                    removed_counts.setdefault(row["method"], Counter())[str(entry["evidence"]["removed_count"])] += 1
            rows.append(entry)
    finally:
        checker.close()
    out = {
        "schema": "position_ab_classes/1", "repo_dir_name": rep["repo_dir_name"], "report": os.path.basename(args.report),
        "servers": [{k: s[k] for k in ("name", "commit", "dirty", "binary") if k in s} for s in rep["servers"]],
        "census": [os.path.basename(p) for p in args.census], "self_containing_references": sum(len(v) for v in contains.values()),
        "rows_differing": len(rows), "summary": dict(sorted(summary.items())),
        "removed_counts_by_method": {m: dict(sorted(c.items())) for m, c in sorted(removed_counts.items())}, "rows": rows,
    }
    Path(args.out).write_text(json.dumps(out, indent=1, sort_keys=False) + "\n", encoding="utf-8")
    print(f"{rep['repo_dir_name']}: differing {len(rows)}, classes {dict(sorted(summary.items()))}; wrote {args.out}")
    return 1 if summary["unclassified"] else 0


if __name__ == "__main__":
    sys.exit(main())
