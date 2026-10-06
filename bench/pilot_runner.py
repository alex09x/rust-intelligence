#!/usr/bin/env python3
"""
rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
Copyright (c) 2026 Alexander Panasenko

Contact: alex@prod.codes
Author: https://prod.codes/about/
Project: https://github.com/alex09x/rust-intelligence
SPDX-License-Identifier: MIT OR Apache-2.0

Pilot and Corpus Benchmark Runner for RIDX vs rust-analyzer.
"""

import argparse
import json
import shutil
import sys
import time
from pathlib import Path

# Add current bench directory to sys.path for local module resolution
bench_dir = Path(__file__).resolve().parent
if str(bench_dir) not in sys.path:
    sys.path.insert(0, str(bench_dir))

from corpus import load_corpus, run_cmd, setup_repo
from metrics import PrometheusCollector, percentile, record_discrepancies
from sampling import sample_scenarios


def run_benchmark_for_repo(
    repo_entry,
    base_dir,
    ridx_bin,
    ra_bin,
    taskset_cpus=None,
    sample_count=30,
    seed=42,
    discrepancies_path=None,
    prom_collector=None,
):
    repo_slug = repo_entry["repo"]
    commit_sha = repo_entry["commit"]
    t_start = time.time()
    result = {
        "repo": repo_slug,
        "commit": commit_sha,
        "status": "PENDING",
        "timestamp": t_start,
    }

    repo_dir, err = setup_repo(repo_entry, base_dir)
    if err:
        result["status"] = "SETUP_FAILED"
        result["error"] = err
        return result

    # 1. Cargo Check: compile build scripts and proc-macros
    check_cmd = ["cargo", "check"]
    code, _, err_out, check_dur = run_cmd(check_cmd, cwd=repo_dir, timeout=600)
    result["cargo_check_sec"] = check_dur
    if code != 0:
        result["status"] = "CARGO_CHECK_FAILED"
        result["error"] = err_out[:500]
        return result

    # 2. Ridx Index: build CAS flat index in target/ridx
    index_cmd = [str(ridx_bin), "index", "."]
    code, _, err_out, index_dur = run_cmd(index_cmd, cwd=repo_dir, timeout=600)
    result["ridx_index_sec"] = index_dur
    if code != 0:
        result["status"] = "RIDX_INDEX_FAILED"
        result["error"] = err_out[:500]
        return result

    # 3. Sample Scenarios
    scenarios = sample_scenarios(repo_dir, ridx_bin, count=sample_count, seed=seed)
    result["scenarios_count"] = len(scenarios)
    if not scenarios:
        result["status"] = "NO_SCENARIOS"
        return result

    scenarios_file = repo_dir / "bench_scenarios.json"
    with open(scenarios_file, "w", encoding="utf-8") as f:
        json.dump(scenarios, f)

    # 4. Run Symmetric Comparison via ridx compare
    cmp_cmd = [
        str(ridx_bin),
        "compare",
        "--scenarios",
        str(scenarios_file),
        "--json",
    ]
    if taskset_cpus:
        cmp_cmd.extend(["--ridx-cmd", f"taskset -c {taskset_cpus} {ridx_bin} lsp"])
        cmp_cmd.extend(["--ra-cmd", f"taskset -c {taskset_cpus} {ra_bin}"])
    else:
        cmp_cmd.extend(["--ra-cmd", str(ra_bin)])
    cmp_cmd.append(str(repo_dir))

    code, out, err_out, cmp_dur = run_cmd(cmp_cmd, cwd=repo_dir, timeout=900)
    if code != 0 or not out:
        result["status"] = "COMPARISON_FAILED"
        result["error"] = err_out[:500] if err_out else "No output from compare"
        return result

    try:
        summary = json.loads(out)
    except Exception as e:
        result["status"] = "PARSE_FAILED"
        result["error"] = f"Failed to parse compare json: {e}"
        return result

    ridx_metrics = summary.get("ridx", {})
    ra_metrics = summary.get("rust_analyzer", {})
    matches = summary.get("scenario_matches", {})

    ridx_lats = ridx_metrics.get("query_latencies_micros", {})
    ra_lats = ra_metrics.get("query_latencies_micros", {})

    cap_latencies = {"def": ([], []), "hover": ([], []), "refs": ([], [])}
    for sc in scenarios:
        s_name = sc["name"]
        cap = sc["capability"]
        if cap in cap_latencies:
            if s_name in ridx_lats:
                cap_latencies[cap][0].append(ridx_lats[s_name])
            if s_name in ra_lats:
                cap_latencies[cap][1].append(ra_lats[s_name])

    metrics = {}
    for cap, (r_list, ra_list) in cap_latencies.items():
        metrics[f"ridx_{cap}_p50_us"] = percentile(r_list, 50)
        metrics[f"ridx_{cap}_p95_us"] = percentile(r_list, 95)
        metrics[f"ridx_{cap}_p99_us"] = percentile(r_list, 99)
        metrics[f"ra_{cap}_p50_us"] = percentile(ra_list, 50)
        metrics[f"ra_{cap}_p95_us"] = percentile(ra_list, 95)
        metrics[f"ra_{cap}_p99_us"] = percentile(ra_list, 99)

    total_scenarios = len(matches)
    matched_scenarios = sum(1 for m in matches.values() if m)
    match_rate = (matched_scenarios / max(1, total_scenarios)) * 100.0

    t_end = time.time()
    collector = prom_collector or PrometheusCollector()
    machine_metrics = collector.collect_machine_metrics(t_start, t_end)

    result.update({
        "status": "SUCCESS",
        "ridx_init_ms": ridx_metrics.get("spawn_and_init_micros", 0) / 1000.0,
        "ridx_warmup_ms": ridx_metrics.get("readiness_warmup_micros", 0) / 1000.0,
        "ridx_peak_rss_mb": ridx_metrics.get("peak_rss_bytes", 0) / (1024 * 1024),
        "ra_init_ms": ra_metrics.get("spawn_and_init_micros", 0) / 1000.0,
        "ra_warmup_ms": ra_metrics.get("readiness_warmup_micros", 0) / 1000.0,
        "ra_peak_rss_mb": ra_metrics.get("peak_rss_bytes", 0) / (1024 * 1024),
        "matched_scenarios": matched_scenarios,
        "total_scenarios": total_scenarios,
        "match_rate_pct": match_rate,
        "metrics": metrics,
        "machine_metrics": machine_metrics,
    })

    if discrepancies_path and matched_scenarios < total_scenarios:
        record_discrepancies(
            discrepancies_path,
            repo_slug,
            scenarios,
            matches,
            ridx_metrics,
            ra_metrics,
        )

    # Clean target directory after both sessions complete to preserve storage
    target_dir = repo_dir / "target"
    if target_dir.exists():
        shutil.rmtree(target_dir, ignore_errors=True)

    return result


def main():
    script_dir = Path(__file__).resolve().parent
    default_corpus = script_dir / "corpus" / "rust-100.tsv"
    default_results = script_dir / "results" / "pilot_results.jsonl"
    default_discrepancies = script_dir / "results" / "discrepancies.jsonl"
    default_ridx = script_dir.parent / "target" / "release" / "ridx"

    parser = argparse.ArgumentParser(description="Pilot and Corpus Benchmark Runner")
    parser.add_argument("--corpus", default=str(default_corpus))
    parser.add_argument("--results", default=str(default_results))
    parser.add_argument("--discrepancies", default=str(default_discrepancies))
    parser.add_argument("--storage", default="/home/alex09x/prod-code-storage/bench_repos")
    parser.add_argument("--ridx", default=str(default_ridx))
    parser.add_argument("--ra", default="rust-analyzer")
    parser.add_argument("--taskset", default="24-31")
    parser.add_argument("--samples", type=int, default=30)
    parser.add_argument("--repo", help="Filter to a single repo (e.g. serde-rs/serde)")
    parser.add_argument("--pilot-only", action="store_true", default=True)
    parser.add_argument("--prometheus-url", help="Prometheus HTTP API URL")
    parser.add_argument("--prometheus-instance", default="ram9", help="Node instance name for Prometheus")
    args = parser.parse_args()

    repos = load_corpus(args.corpus)
    pilot_repos = [
        "tokio-rs/tokio",
        "BurntSushi/ripgrep",
        "serde-rs/serde",
        "bevyengine/bevy",
        "rust-lang/rust-analyzer",
    ]

    if args.repo:
        target_repos = [r for r in repos if r["repo"] == args.repo]
    else:
        target_repos = [r for r in repos if r["repo"] in pilot_repos] if args.pilot_only else repos

    storage_dir = Path(args.storage)
    storage_dir.mkdir(parents=True, exist_ok=True)
    results_path = Path(args.results)
    results_path.parent.mkdir(parents=True, exist_ok=True)
    discrepancies_path = Path(args.discrepancies)
    discrepancies_path.parent.mkdir(parents=True, exist_ok=True)

    ridx_bin = Path(args.ridx).resolve()
    prom_collector = PrometheusCollector(
        endpoint=args.prometheus_url,
        instance=args.prometheus_instance,
    )

    print(f"Running benchmark on {len(target_repos)} repos on node...")
    print(f"Storage: {storage_dir}, Results: {results_path}")

    with open(results_path, "a", encoding="utf-8") as out_f:
        for r in target_repos:
            print(f"\n---> Starting: {r['repo']} (sha: {r['commit'][:8]})")
            res = run_benchmark_for_repo(
                r,
                storage_dir,
                ridx_bin,
                args.ra,
                taskset_cpus=args.taskset,
                sample_count=args.samples,
                discrepancies_path=discrepancies_path,
                prom_collector=prom_collector,
            )
            print(f"---> Finished {r['repo']}: status={res.get('status')}")
            out_f.write(json.dumps(res) + "\n")
            out_f.flush()

    print("\nBenchmark pass completed!")


if __name__ == "__main__":
    main()
