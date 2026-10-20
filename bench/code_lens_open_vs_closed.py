#!/usr/bin/env python3
#
# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
#

"""Compares textDocument/codeLens of ridx binaries on a file closed and on the same file after
didOpen with its on-disk text, and measures the warm latency of both.

    code_lens_open_vs_closed.py --repo DIR --ridx base=BIN [--ridx fix=BIN ...] \\
        --file REL [--file REL ...] [--warm 20] --out FILE

For every binary and file: one request on the closed file (cold), `--warm` more whose latencies
are kept, then `textDocument/didOpen` with the text of the file on disk (version 1), one request
(cold) and `--warm` more, then `didClose`. The last warm answer of each side is kept as a multiset
of (range, name) lenses. The output holds every row (the two multisets compared: common, only
closed, only open, the names behind the differences; warm percentiles per side), a summary per
binary (files whose open answer equals the closed one, warm percentiles over every warm request of
the corpus) and, per pair of binaries, the files whose open answers differ and how many open
answers of the second binary equal the closed answers of the first.
"""

import argparse
import json
import socket
import subprocess
import sys
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from code_lens_latency import lens_key, version_of  # noqa: E402
from lsp_process import SimpleLspProcess  # noqa: E402
from matrix_provenance import check_ridx_index, normalize_repo_identifier  # noqa: E402
from metrics import percentile  # noqa: E402

TOP_NAMES = 6


def parse_version(text):
    """`ridx 0.1.0 (commit: <sha>, dirty: false)` to its commit and dirty flag."""
    commit, dirty = None, None
    inside = text[text.find("(") + 1:text.rfind(")")] if "(" in text and ")" in text else ""
    for part in inside.split(","):
        key, _, value = part.strip().partition(":")
        if key.strip() == "commit":
            commit = value.strip() or None
        elif key.strip() == "dirty":
            dirty = {"true": True, "false": False}.get(value.strip().lower())
    return {"version": text, "commit": commit, "dirty": dirty}


def side_stats(samples):
    """Warm percentiles of one side (closed or open) of one file or one corpus."""
    return {"n": len(samples), "warm_p50_us": round(percentile(samples, 50), 1) if samples else None,
            "warm_p99_us": round(percentile(samples, 99), 1) if samples else None}


def compare_multisets(closed, opened):
    """Both answers as (range, name) multisets: what they share and what each side has alone."""
    only_closed, only_open = closed - opened, opened - closed
    return {
        "closed_lenses": sum(closed.values()), "open_lenses": sum(opened.values()),
        "common": sum((closed & opened).values()),
        "only_closed": sum(only_closed.values()), "only_open": sum(only_open.values()),
        "identical": closed == opened,
        "only_closed_names_top": Counter(k[4] for k in only_closed.elements()).most_common(TOP_NAMES),
        "only_open_names_top": Counter(k[4] for k in only_open.elements()).most_common(TOP_NAMES),
    }


def summarize(rows, closed_samples, open_samples):
    ok = [r for r in rows if r["error"] is None]
    return {
        "files": len(rows), "errors": len(rows) - len(ok),
        "identical_files": sum(1 for r in ok if r["identical"]),
        "closed_lenses": sum(r["closed_lenses"] for r in ok), "open_lenses": sum(r["open_lenses"] for r in ok),
        "closed": side_stats(closed_samples), "open": side_stats(open_samples),
    }


def compare_binaries(answers):
    """`answers[name][file] = (closed multiset, open multiset)`; per pair of binaries, the files
    whose open answers differ and how many open answers of the second equal the closed answers of
    the first (the index composition carried over to the open file)."""
    names = list(answers)
    pairs = []
    for i, a in enumerate(names):
        for b in names[i + 1:]:
            files = sorted(set(answers[a]) | set(answers[b]))
            open_differs = [f for f in files if answers[a].get(f, (None, None))[1] != answers[b].get(f, (None, None))[1]]
            open_b_is_closed_a = sum(1 for f in files if f in answers[a] and f in answers[b]
                                     and answers[b][f][1] == answers[a][f][0])
            pairs.append({"a": a, "b": b, "files": len(files), "open_differing": open_differs,
                          "open_b_equals_closed_a": open_b_is_closed_a})
    return {"binaries": names, "pairs": pairs}


def ask(srv, params, warm):
    """One cold request, then `warm` more; returns the last answer, the cold latency, the warm
    latencies and the first error."""
    res, cold_us, err = srv.request("textDocument/codeLens", params, timeout=60.0)
    samples = []
    for _ in range(warm):
        res_w, us, err_w = srv.request("textDocument/codeLens", params, timeout=60.0)
        if err_w is None:
            res, samples = res_w, samples + [round(us, 1)]
        err = err or err_w
    return res, round(cold_us, 1), samples, err


def measure_file(srv, repo_dir, rel, warm):
    path = Path(repo_dir) / rel
    uri = f"file://{path}"
    params = {"textDocument": {"uri": uri}}
    text = path.read_text(encoding="utf-8", errors="replace")
    closed, closed_cold, closed_warm, e1 = ask(srv, params, warm)
    srv.notify("textDocument/didOpen", {"textDocument": {"uri": uri, "languageId": "rust", "version": 1, "text": text}})
    opened, open_cold, open_warm, e2 = ask(srv, params, warm)
    srv.notify("textDocument/didClose", {"textDocument": {"uri": uri}})
    cc = Counter(lens_key(lens) for lens in closed) if isinstance(closed, list) else Counter()
    co = Counter(lens_key(lens) for lens in opened) if isinstance(opened, list) else Counter()
    row = {"file": rel, "bytes": len(text.encode("utf-8")), "lines": len(text.splitlines()),
           "error": None if (e1 is None and e2 is None) else str(e1 or e2),
           "closed_cold_us": closed_cold, "open_cold_us": open_cold,
           "closed": side_stats(closed_warm), "open": side_stats(open_warm), **compare_multisets(cc, co)}
    return row, (cc, co), closed_warm, open_warm


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--repo", required=True, type=Path)
    ap.add_argument("--ridx", action="append", required=True, metavar="NAME=BIN", help="ridx binary to measure; repeatable")
    ap.add_argument("--file", action="append", required=True, metavar="REL", help="file of the repository; repeatable")
    ap.add_argument("--warm", type=int, default=20, help="warm requests per side and file")
    ap.add_argument("--out", required=True, type=Path)
    args = ap.parse_args()

    repo_dir = args.repo.resolve()
    shards = check_ridx_index(repo_dir)
    started = datetime.now(timezone.utc).isoformat()
    runs, answers = [], {}
    for name, binary in (spec.split("=", 1) for spec in args.ridx):
        print(f"{repo_dir.name}: ridx {name} ({binary}) ...", flush=True)
        srv = SimpleLspProcess([binary, "lsp"], str(repo_dir), is_ra=False)
        rows, closed_samples, open_samples = [], [], []
        try:
            for rel in args.file:
                row, multisets, cw, ow = measure_file(srv, repo_dir, rel, args.warm)
                rows.append(row)
                answers.setdefault(name, {})[rel] = multisets
                closed_samples += cw
                open_samples += ow
                print(f"  {rel:55} closed {row['closed_lenses']:5} open {row['open_lenses']:5} common {row['common']:5} "
                      f"warm p50 {row['closed']['warm_p50_us']}/{row['open']['warm_p50_us']}", flush=True)
        finally:
            srv.close()
        runs.append({"name": name, "binary": "/".join(Path(binary).parts[-3:]), **parse_version(version_of(binary)),
                     "summary": summarize(rows, closed_samples, open_samples), "rows": rows})

    report = {
        "repo": normalize_repo_identifier(repo_dir), "repo_dir_name": repo_dir.name,
        "repo_head": subprocess.run(["git", "-C", str(repo_dir), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip(),
        "node": socket.gethostname(), "python": sys.version.split()[0], "index_shards": shards,
        "warm_requests": args.warm, "files": args.file,
        "timestamp_start": started, "timestamp_end": datetime.now(timezone.utc).isoformat(),
        "runs": runs, "comparison": compare_binaries(answers),
        "metric_definitions": {
            "closed": "textDocument/codeLens on the file before didOpen: one cold request, then warm_requests more",
            "open": "the same after textDocument/didOpen with the text of the file on disk, version 1",
            "warm_p50_us, warm_p99_us": "percentiles over the warm requests of the side (per file, or per corpus in summary)",
            "identical": "the last warm answers of both sides are the same (range, name) multiset",
            "common, only_closed, only_open": "multiset intersection and differences of the two last warm answers",
            "comparison.pairs": "per pair of binaries: files whose open answers differ, and the number of files whose "
                                "open answer of the second binary equals the closed answer of the first",
        },
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=1), encoding="utf-8")
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
