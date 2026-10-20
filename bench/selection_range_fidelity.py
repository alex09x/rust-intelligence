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

"""Measures textDocument/selectionRange fidelity of ridx against rust-analyzer on one corpus.

For the `selection_range` positions of the benchmark matrix (bench/matrix_sampling.py, seed 42,
300 per corpus) it records the whole chain answered by rust-analyzer and by each ridx binary
given, then reports per ridx binary: chain invariant violations, how many innermost ridx nodes
and how many of all ridx nodes also occur in the rust-analyzer chain, how the start of `fn`
item nodes compares with rust-analyzer, and cold/warm latency percentiles.

    selection_range_fidelity.py --repo DIR --ra BIN --ridx before=BIN --ridx after=BIN --out FILE
"""

import argparse
import json
import re
import socket
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from lsp_process import SimpleLspProcess  # noqa: E402
from matrix_canary import ensure_ra_ready  # noqa: E402
from matrix_categories import build_params  # noqa: E402
from matrix_provenance import check_ridx_index, normalize_repo_identifier  # noqa: E402
from matrix_sampling import sample_full_lsp_matrix  # noqa: E402
from metrics import percentile  # noqa: E402

FN_ITEM_PREFIXES = ("fn ", "pub ", "pub(", "const fn", "async ", "unsafe ", "extern ", "default ")


def chain_ranges(result):
    """Flattens one SelectionRange answer into [(sl, sc, el, ec), ...], innermost first."""
    out = []
    node = result[0] if isinstance(result, list) and result else None
    while isinstance(node, dict):
        r = node.get("range") or {}
        s, e = r.get("start") or {}, r.get("end") or {}
        out.append((s.get("line"), s.get("character"), e.get("line"), e.get("character")))
        node = node.get("parent")
    return out


def invariant_violations(chain, cursor):
    """Innermost node must contain the cursor; every parent must strictly contain its child."""
    if not chain:
        return 1
    bad = 0
    sl, sc, el, ec = chain[0]
    if not ((sl, sc) <= cursor <= (el, ec)):
        bad += 1
    for child, parent in zip(chain, chain[1:]):
        contains = (parent[0], parent[1]) <= (child[0], child[1]) and (parent[2], parent[3]) >= (child[2], child[3])
        if not contains or parent == child:
            bad += 1
    return bad


def relative_file(uri, repo_dir):
    """Path of `uri` relative to the corpus root, so results carry no machine-specific prefix."""
    path = Path(uri.replace("file://", "", 1))
    try:
        return str(path.relative_to(repo_dir))
    except ValueError:
        return path.name


def version_of(binary):
    try:
        out = subprocess.run([str(binary), "--version"], capture_output=True, text=True, timeout=30)
        return (out.stdout or out.stderr).strip()
    except Exception as exc:  # pragma: no cover - provenance only
        return f"unavailable: {exc}"


def query_positions(srv, scenarios, repo_dir, is_ra):
    """Asks twice per position (cold, then warm) and keeps the chain of the warm answer."""
    rows = []
    for sc in scenarios:
        params = build_params("selection_range", sc, repo_dir, is_ra=is_ra)
        pos = params["positions"][0]
        res_cold, cold_us, err_cold = srv.request("textDocument/selectionRange", params, timeout=10.0)
        res_warm, warm_us, err_warm = srv.request("textDocument/selectionRange", params, timeout=10.0)
        err = err_warm or err_cold
        rows.append({
            "name": sc["name"],
            "file": relative_file(params["textDocument"]["uri"], repo_dir),
            "position": [pos["line"], pos["character"]],
            "chain": chain_ranges(res_warm if not err_warm else res_cold),
            "error": None if err is None else str(err),
            "cold_us": round(cold_us, 1),
            "warm_us": round(warm_us, 1),
        })
    return rows


def fn_item_start_classes(rows, ra_by_name, repo_dir, file_cache):
    """Classifies every ridx `fn` item node (the parent of a `{..}` body node ending at the same place).

    Returns counts of item nodes found verbatim in rust-analyzer's chain, item nodes not found whose
    alternative start at the first `fn`/`pub`/... keyword line would have been found, and neither.
    """
    counts = {"items": 0, "start_in_ra": 0, "keyword_start_in_ra": 0, "neither": 0}
    for row in rows:
        ra = set(ra_by_name.get(row["name"], []))
        lines = file_cache.setdefault(
            row["file"], (Path(repo_dir) / row["file"]).read_text(encoding="utf-8", errors="replace").split("\n")
        )
        for child, parent in zip(row["chain"], row["chain"][1:]):
            if (parent[2], parent[3]) != (child[2], child[3]) or (parent[0], parent[1]) >= (child[0], child[1]):
                continue
            body_line = lines[child[0]] if child[0] < len(lines) else ""
            if body_line.encode("utf-16-le")[child[1] * 2:child[1] * 2 + 2] != "{".encode("utf-16-le"):
                continue
            counts["items"] += 1
            if parent in ra:
                counts["start_in_ra"] += 1
                continue
            alt = None
            for line_no in range(child[0], parent[0] - 1, -1):
                stripped = lines[line_no].lstrip() if line_no < len(lines) else ""
                if stripped.startswith(FN_ITEM_PREFIXES):
                    alt = (line_no, len(lines[line_no]) - len(stripped), parent[2], parent[3])
                    break
            counts["keyword_start_in_ra" if alt in ra else "neither"] += 1
    return counts


def summarize(rows, ra_rows, repo_dir):
    ra_by_name = {r["name"]: r["chain"] for r in ra_rows}
    ra_first_nonempty = {}
    for r in ra_rows:
        ra_first_nonempty[r["name"]] = next((n for n in r["chain"] if (n[0], n[1]) != (n[2], n[3])), None)
    innermost_in_ra = all_in_ra = innermost_eq_first_nonempty = nodes = nodes_in_ra = violations = errors = 0
    for row in rows:
        if row["error"] or not row["chain"]:
            errors += 1
            continue
        ra = set(ra_by_name.get(row["name"], []))
        violations += invariant_violations(row["chain"], tuple(row["position"]))
        innermost_in_ra += row["chain"][0] in ra
        innermost_eq_first_nonempty += row["chain"][0] == ra_first_nonempty.get(row["name"])
        hits = sum(1 for n in row["chain"] if n in ra)
        nodes += len(row["chain"])
        nodes_in_ra += hits
        all_in_ra += hits == len(row["chain"])
    cold = [r["cold_us"] for r in rows if not r["error"]]
    warm = [r["warm_us"] for r in rows if not r["error"]]
    return {
        "positions": len(rows),
        "errors_or_empty": errors,
        "invariant_violations": violations,
        "innermost_in_ra": innermost_in_ra,
        "innermost_eq_ra_first_nonempty": innermost_eq_first_nonempty,
        "all_in_ra": all_in_ra,
        "nodes": nodes,
        "nodes_in_ra": nodes_in_ra,
        "fn_item_start": fn_item_start_classes(rows, ra_by_name, repo_dir, {}),
        **{f"cold_p{p}_us": percentile(cold, p) for p in (50, 90, 99)},
        **{f"warm_p{p}_us": percentile(warm, p) for p in (50, 90, 99)},
    }


def run_server(cmd, repo_dir, scenarios, canary, is_ra):
    srv = SimpleLspProcess(cmd, repo_dir, is_ra=is_ra)
    ready = {}
    try:
        if is_ra:
            ready = ensure_ra_ready(srv, canary, repo_dir, timeout=900.0, interval=1.0)
        rows = query_positions(srv, scenarios, repo_dir, is_ra)
    finally:
        srv.close()
    return rows, {"initial_wait_ms": round(srv.initial_wait_ms, 1), "initial_quiescent": srv.initial_quiescent, **ready}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--repo", required=True, type=Path)
    ap.add_argument("--ra", required=True, type=Path, help="rust-analyzer binary")
    ap.add_argument("--ridx", action="append", required=True, metavar="NAME=BIN", help="ridx binary to measure; repeatable")
    ap.add_argument("--count", type=int, default=300)
    ap.add_argument("--seed", type=int, default=42)
    ap.add_argument("--out", required=True, type=Path)
    args = ap.parse_args()

    repo_dir = args.repo.resolve()
    shards = check_ridx_index(repo_dir)
    ridx_bins = [(name, Path(binary)) for name, binary in (spec.split("=", 1) for spec in args.ridx)]
    started = datetime.now(timezone.utc).isoformat()

    scenarios_all = sample_full_lsp_matrix(str(repo_dir), per_method=args.count, seed=args.seed)
    scenarios = scenarios_all["selection_range"][: args.count]
    canary = scenarios_all.get("definition", [])[:3]
    print(f"{repo_dir.name}: {len(scenarios)} positions, {shards} index shards", flush=True)

    print("rust-analyzer ...", flush=True)
    ra_rows, ra_ready = run_server([str(args.ra)], repo_dir, scenarios, canary, is_ra=True)
    ra_ok = sum(1 for r in ra_rows if r["chain"] and not r["error"])
    print(f"  answered {ra_ok}/{len(ra_rows)} after {ra_ready}", flush=True)

    servers = {}
    for name, binary in ridx_bins:
        print(f"ridx {name} ({binary}) ...", flush=True)
        rows, ready = run_server([str(binary), "lsp"], repo_dir, scenarios, canary, is_ra=False)
        servers[name] = {
            "binary": "/".join(binary.parts[-3:]),
            "version": version_of(binary),
            "ready": ready,
            "summary": summarize(rows, ra_rows, repo_dir),
            "rows": rows,
        }
        print(f"  {json.dumps(servers[name]['summary'])}", flush=True)

    report = {
        "repo": normalize_repo_identifier(repo_dir),
        "repo_dir_name": repo_dir.name,
        "repo_head": subprocess.run(["git", "-C", str(repo_dir), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip(),
        "node": socket.gethostname(),
        "python": sys.version.split()[0],
        "seed": args.seed,
        "count": len(scenarios),
        "index_shards": shards,
        "timestamp_start": started,
        "timestamp_end": datetime.now(timezone.utc).isoformat(),
        "rust_analyzer": {"binary": str(args.ra), "version": version_of(args.ra), "ready": ra_ready, "answered": ra_ok, "rows": ra_rows},
        "ridx": servers,
        "metric_definitions": {
            "invariant_violations": "innermost node not containing the cursor, or a parent not strictly containing its child",
            "innermost_in_ra": "positions whose innermost ridx range occurs anywhere in the rust-analyzer chain",
            "innermost_eq_ra_first_nonempty": "positions whose innermost ridx range equals rust-analyzer's first non-empty node",
            "all_in_ra": "positions where every ridx range occurs in the rust-analyzer chain",
            "nodes_in_ra": "ridx ranges occurring in the rust-analyzer chain, over all positions",
            "fn_item_start": "ridx fn item nodes (parent of a `{` body node with the same end): found in rust-analyzer as is, "
                             "found only with the start moved to the first fn/pub/... keyword line, or neither",
            "latency": "per-request round trip measured by the client, cold = first ask, warm = second ask of the same position",
        },
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=1), encoding="utf-8")
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
