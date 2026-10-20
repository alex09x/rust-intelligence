# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Asks several servers the requests of a published matrix run, position by position, and
compares their answers.

    position_ab.py --repo DIR --positions bench/published/matrix_<corpus>_booster.positions.json \\
        --ridx NAME=BIN [--ridx NAME=BIN ...] [--ra NAME=BIN] [--methods key,key,...] \\
        [--limit N] [--timeout 2.0] [--matrix bench/published/matrix_<corpus>_booster.json] --out FILE \\
        [--out-differing FILE]

The positions file fixes the requests: every record is rebuilt into the scenario the sampler
produced, so `matrix_categories.build_params` yields the request of the matrix. Each server (ridx
binaries first, then rust-analyzer, one process each) answers every selected request method in
matrix order, one request per position, with a drain and a barrier between methods and, for
rust-analyzer, the matrix's readiness wait after initialize. Per position the output records the
outcome by the matrix rule (a non-empty, b empty, c error), a digest of the answer (completion
items compared as a set), the latency, the cargo target kind of the file (from the index's
target_kinds.json, a path heuristic) and the kind of token under the cursor, including whether it
sits under `#[cfg(test)]`. Per server and method: a/b/c counts and p50/p99, optionally checked
against the a/b/c of a published matrix JSON. Per pair of servers and method: positions with the
same answer, differing answers, the outcome transitions and the first differing positions.
`--out-differing` writes a second copy that keeps the summaries and the comparison whole but only the
rows on which some pair of servers differs, with `rows_total` and the sha256 of the full file.
"""

import argparse
import hashlib
import json
import socket
import subprocess
import sys
import time
from collections import Counter
from datetime import datetime, timezone
from itertools import combinations
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from code_lens_latency import version_of  # noqa: E402
from code_lens_open_vs_closed import parse_version  # noqa: E402
from lsp_barrier import barrier  # noqa: E402
from lsp_process import SimpleLspProcess  # noqa: E402
from matrix_canary import ensure_ra_ready  # noqa: E402
from matrix_categories import LSP_SPECS, build_params  # noqa: E402
from matrix_provenance import check_ridx_index, normalize_repo_identifier  # noqa: E402
from metrics import percentile  # noqa: E402
from position_ab_classify import (  # noqa: E402
    REQUEST_KEYS, classify_outcome, cursor_kind, digest, load_target_entries, scenario_from_record,
    target_kind_of,
)

RPC_OF = {key: rpc for _, key, rpc, _, _ in LSP_SPECS}
EXAMPLES = 10


def build_rows(positions, methods, limit, repo_dir, entries, read_lines):
    """One row per selected position, in matrix order, with its scenario attached."""
    rows = []
    for key in methods:
        for rec in (positions.get(key) or [])[:limit]:
            rel = rec.get("file") or ""
            cursor = {"cursor_kind": "no_file", "cfg_test": False}
            if rel and rec.get("line"):
                cursor = cursor_kind(read_lines(rel), int(rec["line"]), int(rec.get("character") or 1), rec.get("symbol"))
            rows.append({
                "method": key, "rpc_method": RPC_OF[key], "name": rec.get("name", ""), "file": rel,
                "line": rec.get("line"), "character": rec.get("character"), "symbol": rec.get("symbol"),
                "target_kind": target_kind_of(rel, entries) if rel else "none", **cursor,
                "scenario": scenario_from_record(key, rec, repo_dir), "results": {},
            })
    return rows


def run_server(name, cmd, is_ra, rows, repo_dir, timeout, canary):
    """Answers every row's request with one server process; fills `rows[i]["results"][name]`."""
    srv = SimpleLspProcess(cmd, str(repo_dir), is_ra=is_ra)
    t0 = time.perf_counter()
    ready = {"initial_wait_ms": round(srv.initial_wait_ms, 1), "initial_quiescent": srv.initial_quiescent}
    try:
        if is_ra:
            ready.update(ensure_ra_ready(srv, canary, Path(repo_dir), timeout=600.0, interval=1.0))
        unsupported = set()
        current = None
        for row in rows:
            key = row["method"]
            if key != current:
                current = key
                srv.drain(timeout=600.0)
                barrier(srv, timeout=600.0)
            if key in unsupported:
                row["results"][name] = {"outcome": "c", "digest": None, "us": 0.0, "error": "-32601"}
                continue
            params = build_params(key, row["scenario"], str(repo_dir), is_ra=is_ra)
            res, us, err = srv.request(row["rpc_method"], params, timeout=timeout)
            outcome, code = classify_outcome(key, res, err)
            if code == "-32601":
                unsupported.add(key)
            row["results"][name] = {
                "outcome": outcome, "digest": digest(key, res) if outcome == "a" else ("empty" if outcome == "b" else None),
                "us": round(us, 1), "error": code,
            }
    finally:
        srv.close()
    ready["cancelled_on_timeout"] = srv.cancelled_on_timeout
    return ready, round(time.perf_counter() - t0, 1)


def summarize_server(rows, name):
    """Per method: a/b/c counts, p50/p99 of the single requests, and whether the server rejected
    the method as unknown."""
    out = {}
    for key in dict.fromkeys(r["method"] for r in rows):
        results = [r["results"][name] for r in rows if r["method"] == key and name in r["results"]]
        counts = Counter(r["outcome"] for r in results)
        us = [r["us"] for r in results if r["outcome"] != "c" or r["error"] != "-32601"]
        out[key] = {"a": counts["a"], "b": counts["b"], "c": counts["c"], "n": len(results),
                    "p50_us": round(percentile(us, 50), 1) if us else None,
                    "p99_us": round(percentile(us, 99), 1) if us else None,
                    "unsupported": any(r["error"] == "-32601" for r in results)}
    return out


def matrix_check(summary, matrix_methods):
    """Compares a server's a/b/c per method with a published matrix side; only methods whose
    position count equals the matrix's tests_run are compared."""
    compared, mismatches = 0, []
    for key, s in summary.items():
        m = matrix_methods.get(key)
        if not m or m.get("tests_run") != s["n"]:
            continue
        compared += 1
        theirs = [m.get("a_non_empty", 0), m.get("b_empty", 0), m.get("c_error", 0)]
        if [s["a"], s["b"], s["c"]] != theirs:
            mismatches.append({"method": key, "ours": [s["a"], s["b"], s["c"]], "matrix": theirs})
    return {"compared": compared, "matching": compared - len(mismatches), "mismatches": mismatches}


def compare_servers(rows, names, examples=EXAMPLES):
    """Per pair of servers and method: positions with the same digest, differing ones, the
    outcome transitions (first server > second) and the first differing positions."""
    pairs = []
    for a, b in combinations(names, 2):
        methods = {}
        for key in dict.fromkeys(r["method"] for r in rows):
            same = differing = 0
            transitions = Counter()
            shown = []
            for r in rows:
                if r["method"] != key or a not in r["results"] or b not in r["results"]:
                    continue
                ra, rb = r["results"][a], r["results"][b]
                if ra["digest"] == rb["digest"] and ra["outcome"] == rb["outcome"]:
                    same += 1
                    continue
                differing += 1
                transitions[f"{ra['outcome']}>{rb['outcome']}"] += 1
                if len(shown) < examples:
                    shown.append({"name": r["name"], "file": r["file"], "line": r["line"], "character": r["character"],
                                  "outcomes": [ra["outcome"], rb["outcome"]], "digests": [ra["digest"], rb["digest"]],
                                  "target_kind": r["target_kind"], "cursor_kind": r["cursor_kind"], "cfg_test": r["cfg_test"]})
            methods[key] = {"same": same, "differing": differing, "transitions": dict(transitions), "examples": shown}
        pairs.append({"a": a, "b": b, "same": sum(m["same"] for m in methods.values()),
                      "differing": sum(m["differing"] for m in methods.values()), "methods": methods})
    return pairs


def row_differs(row):
    """True when two servers answered the row with a different outcome or digest."""
    seen = {(r["outcome"], r["digest"]) for r in row["results"].values()}
    return len(seen) > 1


def differing_only(report, full_name, full_sha256):
    """The report with only the differing rows; summaries and comparison stay whole, `rows_total`
    and the digest of the full file tie the copy to the full run."""
    kept = [row for row in report["rows"] if row_differs(row)]
    out = {k: v for k, v in report.items() if k != "rows"}
    out.update({"rows": kept, "rows_total": len(report["rows"]), "rows_kept": len(kept), "rows_filter": "differing",
                "full_file": {"name": full_name, "sha256": full_sha256}})
    out["metric_definitions"] = {**report["metric_definitions"],
                                 "rows_filter": "differing: only rows on which some pair of servers differs in outcome or digest; "
                                                "rows_total counts the rows of the full file named in full_file"}
    return out


def parse_servers(ridx_specs, ra_specs):
    servers = [(name, [binary, "lsp"], False, binary) for name, binary in (s.split("=", 1) for s in ridx_specs or [])]
    servers += [(name, [binary], True, binary) for name, binary in (s.split("=", 1) for s in ra_specs or [])]
    return servers


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--repo", required=True, type=Path)
    ap.add_argument("--positions", required=True, type=Path, help="published matrix positions JSON")
    ap.add_argument("--ridx", action="append", metavar="NAME=BIN", help="ridx binary; repeatable")
    ap.add_argument("--ra", action="append", metavar="NAME=BIN", help="rust-analyzer binary; repeatable")
    ap.add_argument("--methods", help="comma-separated matrix keys; default: every request method with positions")
    ap.add_argument("--limit", type=int, default=None, help="positions per method; default: all")
    ap.add_argument("--timeout", type=float, default=2.0, help="seconds per request, as in the matrix")
    ap.add_argument("--matrix", type=Path, help="published matrix JSON to check the a/b/c counts against")
    ap.add_argument("--out", required=True, type=Path, help="full report with every row")
    ap.add_argument("--out-differing", type=Path, help="copy of the report keeping only the rows on which some pair of servers differs, with rows_total and the sha256 of the full file")
    args = ap.parse_args()
    servers = parse_servers(args.ridx, args.ra)
    if not servers:
        ap.error("at least one --ridx or --ra is required")

    repo_dir = args.repo.resolve()
    shards = check_ridx_index(repo_dir)
    payload = json.loads(args.positions.read_text(encoding="utf-8"))
    positions = payload["positions"]
    methods = args.methods.split(",") if args.methods else [k for k in REQUEST_KEYS if positions.get(k)]
    unknown = [m for m in methods if m not in REQUEST_KEYS]
    if unknown:
        ap.error(f"not request methods of the matrix: {unknown}")
    entries = load_target_entries(repo_dir)
    cache = {}

    def read_lines(rel):
        if rel not in cache:
            try:
                cache[rel] = (repo_dir / rel).read_text(encoding="utf-8", errors="replace").split("\n")
            except OSError:
                cache[rel] = []
        return cache[rel]

    rows = build_rows(positions, methods, args.limit, repo_dir, entries, read_lines)
    canary = [scenario_from_record("definition", rec, repo_dir) for rec in (positions.get("definition") or [])[:3]]
    matrix = json.loads(args.matrix.read_text(encoding="utf-8")) if args.matrix else None
    started = datetime.now(timezone.utc).isoformat()
    runs = []
    for name, cmd, is_ra, binary in servers:
        print(f"{repo_dir.name}: {'rust-analyzer' if is_ra else 'ridx'} {name} ({binary}), {len(rows)} requests ...", flush=True)
        ready, seconds = run_server(name, cmd, is_ra, rows, repo_dir, args.timeout, canary)
        summary = summarize_server(rows, name)
        run = {"name": name, "kind": "rust-analyzer" if is_ra else "ridx", "binary": "/".join(Path(binary).parts[-3:]),
               **parse_version(version_of(binary)), "ready": ready, "duration_s": seconds, "summary": summary}
        if matrix:
            run["matrix_check"] = matrix_check(summary, matrix.get("rust_analyzer" if is_ra else "ridx", {}).get("methods", {}))
            print(f"  matrix check: {run['matrix_check']['matching']} of {run['matrix_check']['compared']} methods match", flush=True)
        for key, s in summary.items():
            print(f"  {key:28s} a/b/c {s['a']:4d}/{s['b']:4d}/{s['c']:4d}  p50 {s['p50_us']} p99 {s['p99_us']}", flush=True)
        runs.append(run)
    for row in rows:
        row.pop("scenario", None)

    comparison = compare_servers(rows, [s[0] for s in servers])
    for pair in comparison:
        print(f"{pair['a']} vs {pair['b']}: same {pair['same']}, differing {pair['differing']}", flush=True)
    report = {
        "repo": normalize_repo_identifier(repo_dir), "repo_dir_name": repo_dir.name,
        "repo_head": subprocess.run(["git", "-C", str(repo_dir), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip(),
        "node": socket.gethostname(), "python": sys.version.split()[0], "index_shards": shards,
        "positions_file": args.positions.name, "positions_seed": payload.get("seed"), "methods": methods,
        "limit": args.limit, "timeout_s": args.timeout, "target_entries": len(entries),
        "timestamp_start": started, "timestamp_end": datetime.now(timezone.utc).isoformat(),
        "servers": runs, "comparison": comparison, "rows": rows,
        "metric_definitions": {
            "outcome": "matrix rule of bench/matrix_execute.py: c on error, b on null / [] / \"\" / {} / hover without contents, else a",
            "digest": "first 16 hex digits of sha256 over the canonical JSON of the answer; completion items sorted first; "
                      "\"empty\" for outcome b, null for outcome c",
            "us": "latency of the single request, microseconds (first request on the position, no warm repeat)",
            "target_kind": "cargo target kind from target_kinds.json of the active index variant, by entry path heuristic",
            "cursor_kind": "token under the cursor by the sampler's own regexes: decl:<fn|struct|enum|trait|type>, impl, "
                           "method_call, path, type_usage, macro, use, let, ident",
            "cfg_test": "line inside a #[cfg(test)] mod block, or the attribute stands directly above it",
            "comparison": "per pair of servers and method: rows with equal outcome and digest, differing rows, "
                          "outcome transitions first>second, and the first differing rows",
            "matrix_check": "a/b/c per method against the published matrix side of the same server kind, "
                            "for methods run on as many positions as the matrix",
        },
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    full_text = json.dumps(report, indent=1)
    args.out.write_text(full_text, encoding="utf-8")
    print(f"wrote {args.out}")
    if args.out_differing:
        digest_full = hashlib.sha256(full_text.encode("utf-8")).hexdigest()
        filtered = differing_only(report, args.out.name, digest_full)
        args.out_differing.parent.mkdir(parents=True, exist_ok=True)
        args.out_differing.write_text(json.dumps(filtered, indent=1), encoding="utf-8")
        print(f"wrote {args.out_differing} ({filtered['rows_kept']} of {filtered['rows_total']} rows)")


if __name__ == "__main__":
    main()
