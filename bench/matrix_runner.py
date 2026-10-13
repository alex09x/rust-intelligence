# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Full Matrix Benchmark Runner for all 78 LSP 3.17 methods across 300 test varieties.
Orchestrates symmetric execution against RIDX and rust-analyzer, recording latency distributions,
memory (Anon/File RSS), FDs, and match fidelity across both Linux and macOS.
"""

import argparse
import json
import os
import shutil
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

bench_dir = Path(__file__).resolve().parent
if str(bench_dir) not in sys.path:
    sys.path.insert(0, str(bench_dir))

from matrix_categories import LSP_SPECS
from matrix_execute import execute_server_suite
from matrix_provenance import check_ridx_index, collect_matrix_provenance, save_scenarios_positions
from matrix_sampling import sample_full_lsp_matrix


def run_full_matrix_benchmark(repo_dir, ridx_bin, ra_bin, sample_count=300, seed=42, out_path=None, repo_id=None):
    """Executes the full 78 methods x sample_count test matrix on both servers."""
    check_ridx_index(repo_dir)
    ts_start = datetime.now(timezone.utc).isoformat()
    print(f"Sampling {sample_count} varieties across all 78 LSP methods for {repo_dir} (seed={seed})...")
    scenarios = sample_full_lsp_matrix(repo_dir, per_method=sample_count, seed=seed)

    if out_path:
        pos_path = Path(out_path).with_suffix(".positions.json")
        save_scenarios_positions(scenarios, pos_path, repo_dir, seed=seed, sample_count=sample_count)
        print(f"Recorded benchmark sample positions to {pos_path}")

    print("Running RIDX test suite across all 78 methods...")
    ridx_cmd = [str(ridx_bin), "lsp"]
    t0 = time.time()
    ridx_res = execute_server_suite(ridx_cmd, repo_dir, scenarios, is_ra=False, sample_count=sample_count)
    ridx_dur = time.time() - t0

    print("Running rust-analyzer test suite across all 78 methods...")
    ra_cmd = [str(ra_bin)]
    t0 = time.time()
    ra_res = execute_server_suite(ra_cmd, repo_dir, scenarios, is_ra=True, sample_count=sample_count)
    ra_dur = time.time() - t0
    ts_end = datetime.now(timezone.utc).isoformat()
    prov = collect_matrix_provenance(repo_dir, ridx_bin, ra_bin, seed, ts_start, ts_end, repo_id=repo_id)

    res = {
        "repo": prov["repo"],
        "sample_count_per_method": sample_count,
        "seed": seed,
        "total_methods": len(LSP_SPECS),
        "total_tests_per_server": sum(v.get("tests_run", 0) for v in ridx_res["methods"].values()),
        "ridx_duration_sec": round(ridx_dur, 2),
        "ra_duration_sec": round(ra_dur, 2),
        "ridx_initial_quiescent": ridx_res.get("initial_quiescent", True),
        "ridx_initial_wait_ms": ridx_res.get("initial_wait_ms", 0.0),
        "ra_initial_quiescent": ra_res.get("initial_quiescent", False),
        "ra_initial_wait_ms": ra_res.get("initial_wait_ms", 0.0),
        "ra_canary_wait_ms": ra_res.get("canary_wait_ms", 0.0),
        "ra_canary_attempts": ra_res.get("canary_attempts", 0),
        "ra_quiescent_pre_query_wait_ms": ra_res.get("quiescent_pre_query_wait_ms", 0.0),
        "ra_canary_pre_query_wait_ms": ra_res.get("canary_pre_query_wait_ms", 0.0),
        "ra_canary_pre_query_attempts": ra_res.get("canary_pre_query_attempts", 0),
        "ra_progress_titles_seen": ra_res.get("progress_titles_seen", []),
        "ra_progress_events": ra_res.get("progress_events", []),
        "ridx_open_documents_before_queries": ridx_res.get("open_documents_before_queries", 0),
        "ra_open_documents_before_queries": ra_res.get("open_documents_before_queries", 0),
        "ridx_cancelled_on_timeout": ridx_res.get("cancelled_on_timeout", 0),
        "ra_cancelled_on_timeout": ra_res.get("cancelled_on_timeout", 0),
    }
    # Every provenance field recorded by collect_matrix_provenance is carried into the result.
    res.update({k: v for k, v in prov.items() if k not in res})
    res["ridx"] = ridx_res
    res["rust_analyzer"] = ra_res
    return res


def main():
    parser = argparse.ArgumentParser(description="Full 78-method LSP matrix benchmark runner")
    parser.add_argument("--repo", required=True, help="Repository root path")
    parser.add_argument("--repo-id", help="Canonical repository identifier (e.g. owner/name)")
    parser.add_argument("--ridx-bin", default="target/release/ridx", help="Path to ridx binary")
    parser.add_argument("--ra-bin", default="rust-analyzer", help="Path to rust-analyzer binary")
    parser.add_argument("--sample-count", type=int, default=300, help="Number of test varieties per method")
    parser.add_argument("--seed", type=int, default=42, help="Random seed for scenario sampling")
    parser.add_argument("--output", default="bench/results/matrix_results.json", help="Output JSON path")
    args = parser.parse_args()

    repo_dir = Path(args.repo).resolve()
    from matrix_provenance import CANONICAL_REPOS
    if repo_dir.name not in CANONICAL_REPOS and not args.repo_id:
        parser.error(f"Unknown benchmark repository '{repo_dir.name}'. Please specify canonical --repo-id owner/name.")

    out_path = Path(args.output)
    results = run_full_matrix_benchmark(
        repo_dir=repo_dir,
        ridx_bin=Path(args.ridx_bin).resolve(),
        ra_bin=shutil.which(str(args.ra_bin)) or (Path(args.ra_bin).resolve() if Path(args.ra_bin).is_file() else args.ra_bin),
        sample_count=args.sample_count,
        seed=args.seed,
        out_path=out_path,
        repo_id=args.repo_id,
    )

    out_path.parent.mkdir(parents=True, exist_ok=True)
    with open(out_path, "w", encoding="utf-8") as f:
        json.dump(results, f, indent=2)
    print(f"Benchmark matrix results saved to {out_path}")


if __name__ == "__main__":
    main()
