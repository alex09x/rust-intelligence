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

from corpus import load_corpus, run_cmd, run_cmd_with_rss, setup_repo
from metrics import PrometheusCollector, is_noise_rejected, percentile, record_discrepancies
from sampling import sample_scenarios


def run_benchmark_for_repo(
    repo_entry,
    base_dir,
    ridx_bin,
    ra_bin,
    taskset_cpus=None,
    sample_count=300,
    w1_steps=100,
    w2_iterations=10,
    w3_counts=(1, 2, 5, 10),
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

    # 1. Cold Cargo Check & Cold Ridx Index (from scratch)
    run_cmd(["cargo", "clean"], cwd=repo_dir)
    check_cmd = ["cargo", "check", "--workspace"]
    code, _, err_out, check_dur, chk_rss, chk_anon, chk_file, chk_fds, chk_ths = run_cmd_with_rss(check_cmd, cwd=repo_dir, timeout=600)
    result["cold_cargo_check_sec"] = check_dur
    result["cargo_check_peak_rss_mb"] = chk_rss / (1024 * 1024)
    result["cargo_check_peak_fds"] = chk_fds
    result["cargo_check_peak_threads"] = chk_ths
    if code != 0:
        result["status"] = "CARGO_CHECK_FAILED"
        result["error"] = err_out[:500]
        return result

    # 2. Cold Ridx Index: build CAS flat index in target/ridx
    index_cmd = [str(ridx_bin), "index", "."]
    code, _, err_out, index_dur, idx_rss, idx_anon, idx_file, idx_fds, idx_ths = run_cmd_with_rss(index_cmd, cwd=repo_dir, timeout=600)
    result["cold_ridx_index_sec"] = index_dur
    result["ridx_index_peak_rss_mb"] = idx_rss / (1024 * 1024)
    result["ridx_index_peak_fds"] = idx_fds
    result["ridx_index_peak_threads"] = idx_ths
    ridx_size = sum(f.stat().st_size for f in repo_dir.glob("target/**/*.ridx") if f.is_file())
    result["ridx_index_size_bytes"] = ridx_size
    if code != 0:
        result["status"] = "RIDX_INDEX_FAILED"
        result["error"] = err_out[:500]
        return result

    # 2b. Warm incremental Ridx Index
    t_warm0 = time.perf_counter()
    run_cmd([str(ridx_bin), "index", "."], cwd=repo_dir, timeout=600)
    result["warm_ridx_index_sec"] = time.perf_counter() - t_warm0

    # 3. Build rust-analyzer SCIP index (static precomputed index from scratch)
    scip_file = repo_dir / "index.scip"
    scip_cmd = [str(ra_bin), "scip", str(repo_dir), "--output", str(scip_file), "--exclude-vendored-libraries"]
    if taskset_cpus:
        scip_cmd = ["taskset", "-c", str(taskset_cpus)] + scip_cmd
    code, _, err_out, scip_dur, scip_rss, scip_anon, scip_file_rss, scip_fds, scip_ths = run_cmd_with_rss(scip_cmd, cwd=repo_dir, timeout=600)
    result["ra_scip_build_sec"] = scip_dur
    result["ra_scip_peak_rss_mb"] = scip_rss / (1024 * 1024)
    result["ra_scip_peak_rss_anon_mb"] = scip_anon / (1024 * 1024)
    result["ra_scip_peak_rss_file_mb"] = scip_file_rss / (1024 * 1024)
    result["ra_scip_peak_fds"] = scip_fds
    result["ra_scip_peak_threads"] = scip_ths
    result["ra_scip_size_bytes"] = scip_file.stat().st_size if scip_file.exists() else 0
    if code != 0:
        result["ra_scip_error"] = err_out[:500] if err_out else "SCIP generation failed"

    # 4. Sample Scenarios
    scenarios = sample_scenarios(repo_dir, count=sample_count, seed=seed)
    result["scenarios_count"] = len(scenarios)
    if not scenarios:
        result["status"] = "NO_SCENARIOS"
        return result

    scenarios_file = repo_dir / "bench_scenarios.json"
    with open(scenarios_file, "w", encoding="utf-8") as f:
        json.dump(scenarios, f)

    # 5. Run Symmetric Comparison via ridx compare
    cmp_cmd = [
        str(ridx_bin),
        "compare",
        "--scenarios",
        str(scenarios_file),
        "--json",
    ]
    if scip_file.exists():
        cmp_cmd.extend(["--scip-file", str(scip_file)])
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
    scip_metrics = summary.get("rust_analyzer_scip")
    matches = summary.get("scenario_matches", {})
    scip_matches = summary.get("scip_matches", {})

    ridx_lats = ridx_metrics.get("query_latencies_micros", {})
    ridx_cold_lats = ridx_metrics.get("query_cold_latencies_micros", {})
    ra_lats = ra_metrics.get("query_latencies_micros", {})
    ra_cold_lats = ra_metrics.get("query_cold_latencies_micros", {})
    scip_lats = scip_metrics.get("query_latencies_micros", {}) if scip_metrics else {}
    scip_cold_lats = scip_metrics.get("query_cold_latencies_micros", {}) if scip_metrics else {}

    cap_latencies = {
        "def": {"warm": ([], [], []), "cold": ([], [], [])},
        "hover": {"warm": ([], [], []), "cold": ([], [], [])},
        "refs": {"warm": ([], [], []), "cold": ([], [], [])},
        "mutation": {"warm": ([], [], []), "cold": ([], [], [])},
    }
    for sc in scenarios:
        s_name = sc["name"]
        cap = "mutation" if sc.get("kind") == "cross_file_mutation" else sc["capability"]
        if cap in cap_latencies:
            if s_name in ridx_lats:
                cap_latencies[cap]["warm"][0].append(ridx_lats[s_name])
            if s_name in ra_lats:
                cap_latencies[cap]["warm"][1].append(ra_lats[s_name])
            if scip_metrics and s_name in scip_lats:
                cap_latencies[cap]["warm"][2].append(scip_lats[s_name])
            if s_name in ridx_cold_lats:
                cap_latencies[cap]["cold"][0].append(ridx_cold_lats[s_name])
            if s_name in ra_cold_lats:
                cap_latencies[cap]["cold"][1].append(ra_cold_lats[s_name])
            if scip_metrics and s_name in scip_cold_lats:
                cap_latencies[cap]["cold"][2].append(scip_cold_lats[s_name])

    metrics = {}
    for cap, mode_dict in cap_latencies.items():
        for mode in ["warm", "cold"]:
            r_list, ra_list, scip_list = mode_dict[mode]
            metrics[f"ridx_{cap}_{mode}_p50_us"] = percentile(r_list, 50)
            metrics[f"ridx_{cap}_{mode}_p90_us"] = percentile(r_list, 90)
            metrics[f"ridx_{cap}_{mode}_p95_us"] = percentile(r_list, 95)
            metrics[f"ridx_{cap}_{mode}_p99_us"] = percentile(r_list, 99)
            metrics[f"ra_{cap}_{mode}_p50_us"] = percentile(ra_list, 50)
            metrics[f"ra_{cap}_{mode}_p90_us"] = percentile(ra_list, 90)
            metrics[f"ra_{cap}_{mode}_p95_us"] = percentile(ra_list, 95)
            metrics[f"ra_{cap}_{mode}_p99_us"] = percentile(ra_list, 99)
            if cap != "mutation":
                metrics[f"scip_{cap}_{mode}_p50_us"] = percentile(scip_list, 50)
                metrics[f"scip_{cap}_{mode}_p90_us"] = percentile(scip_list, 90)
                metrics[f"scip_{cap}_{mode}_p95_us"] = percentile(scip_list, 95)
                metrics[f"scip_{cap}_{mode}_p99_us"] = percentile(scip_list, 99)
            else:
                metrics[f"scip_{cap}_{mode}_p50_us"] = "N/A"

    matches_line = summary.get("scenario_matches_line_level", {})
    matched_scenarios_strict = sum(1 for m in matches.values() if m)
    matched_scenarios_line = sum(1 for m in matches_line.values() if m)
    matched_scenarios = matched_scenarios_strict
    total_scenarios = len(matches)
    match_rate_strict = (matched_scenarios_strict / max(1, total_scenarios)) * 100.0
    match_rate_line = (matched_scenarios_line / max(1, total_scenarios)) * 100.0

    mut_names = set(summary.get("mutation_verifications", {}).keys())
    ro_strict = sum(1 for k, m in matches.items() if k not in mut_names and m)
    ro_line = sum(1 for k, m in matches_line.items() if k not in mut_names and m)
    ro_total = max(1, len([k for k in matches if k not in mut_names]))
    ro_strict_pct = (ro_strict / ro_total) * 100.0
    ro_line_pct = (ro_line / ro_total) * 100.0

    t_end = time.time()
    collector = prom_collector or PrometheusCollector()
    machine_metrics = collector.collect_machine_metrics(t_start, t_end)
    is_noisy, noise_reason = is_noise_rejected(machine_metrics)

    result.update({
        "status": "NOISE_REJECTED" if is_noisy else "SUCCESS",
        "noise_reason": noise_reason,
        "ridx_init_ms": ridx_metrics.get("spawn_and_init_micros", 0) / 1000.0,
        "ridx_warmup_ms": ridx_metrics.get("readiness_warmup_micros", 0) / 1000.0,
        "ridx_peak_rss_mb": ridx_metrics.get("peak_rss_bytes", 0) / (1024 * 1024),
        "ridx_peak_rss_anon_mb": ridx_metrics.get("peak_rss_anon_bytes", 0) / (1024 * 1024),
        "ridx_peak_rss_file_mb": ridx_metrics.get("peak_rss_file_bytes", 0) / (1024 * 1024),
        "ridx_peak_open_fds": ridx_metrics.get("peak_open_fds", 0),
        "ridx_peak_threads": ridx_metrics.get("peak_threads", 0),
        "ridx_peak_ridx_open_fds": ridx_metrics.get("peak_ridx_open_fds", 0),
        "ridx_peak_ridx_mapped_regions": ridx_metrics.get("peak_ridx_mapped_regions", 0),
        "ra_init_ms": ra_metrics.get("spawn_and_init_micros", 0) / 1000.0,
        "ra_warmup_ms": ra_metrics.get("readiness_warmup_micros", 0) / 1000.0,
        "ra_peak_rss_mb": ra_metrics.get("peak_rss_bytes", 0) / (1024 * 1024),
        "ra_peak_rss_anon_mb": ra_metrics.get("peak_rss_anon_bytes", 0) / (1024 * 1024),
        "ra_peak_rss_file_mb": ra_metrics.get("peak_rss_file_bytes", 0) / (1024 * 1024),
        "ra_peak_open_fds": ra_metrics.get("peak_open_fds", 0),
        "ra_peak_threads": ra_metrics.get("peak_threads", 0),
        "cargo_spawned_during_queries": ridx_metrics.get("cargo_spawned", False) or ra_metrics.get("cargo_spawned", False),
        "ridx_cargo_spawned": ridx_metrics.get("cargo_spawned", False),
        "ra_cargo_spawned": ra_metrics.get("cargo_spawned", False),
        "readonly_total": ro_total,
        "readonly_matched_strict": ro_strict,
        "readonly_match_rate_strict_pct": ro_strict_pct,
        "readonly_matched_line_level": ro_line,
        "readonly_match_rate_line_level_pct": ro_line_pct,
        "matched_scenarios_strict": matched_scenarios_strict,
        "match_rate_strict_pct": match_rate_strict,
        "matched_scenarios_line_level": matched_scenarios_line,
        "match_rate_line_level_pct": match_rate_line,
        "matched_scenarios": matched_scenarios_strict,
        "total_scenarios": total_scenarios,
        "match_rate_pct": match_rate_strict,
        "metrics": metrics,
        "machine_metrics": machine_metrics,
        "mutation_verifications": summary.get("mutation_verifications", {}),
    })

    # 6. Run Worktree Scenarios (W1, W2, W3)
    try:
        from worktree_bench import run_scenario_w1_commit_walk, run_scenario_w2_large_jumps, run_scenario_w3_multi_worktree
        result["w1_commit_walk"] = run_scenario_w1_commit_walk(repo_dir, ridx_bin, ra_bin, steps=w1_steps)
        result["w2_large_jumps"] = run_scenario_w2_large_jumps(repo_dir, ridx_bin, iterations=w2_iterations)
        result["w3_multi_worktree"] = run_scenario_w3_multi_worktree(repo_dir, ridx_bin, ra_bin=ra_bin, worktree_counts=w3_counts)
    except Exception as e:
        result["worktree_scenarios_error"] = str(e)

    if scip_metrics:
        scip_total = len(scip_matches)
        scip_matched = sum(1 for m in scip_matches.values() if m)
        result.update({
            "scip_load_ms": scip_metrics.get("readiness_warmup_micros", 0) / 1000.0,
            "scip_peak_rss_mb": scip_metrics.get("peak_rss_bytes", 0) / (1024 * 1024),
            "scip_peak_open_fds": scip_metrics.get("peak_open_fds", 0),
            "scip_peak_threads": scip_metrics.get("peak_threads", 0),
            "scip_matched_scenarios": scip_matched,
            "scip_total_scenarios": scip_total,
            "scip_match_rate_pct": (scip_matched / max(1, scip_total)) * 100.0 if scip_total > 0 else 0.0,
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

    # Clean target directory and scip file after both sessions complete to preserve storage
    target_dir = repo_dir / "target"
    if target_dir.exists():
        shutil.rmtree(target_dir, ignore_errors=True)
    if scip_file.exists():
        try:
            scip_file.unlink()
        except OSError:
            pass

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
    parser.add_argument("--samples", type=int, default=300)
    parser.add_argument("--w1-steps", type=int, default=100)
    parser.add_argument("--w2-iterations", type=int, default=10)
    parser.add_argument("--w3-counts", nargs="+", type=int, default=[1, 2, 5, 10])
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
    driver_bin = ridx_bin.parent / "ridx-driver"
    if driver_bin.exists():
        import os
        os.environ["RIDX_DRIVER_BIN"] = str(driver_bin)
    prom_collector = PrometheusCollector(
        endpoint=args.prometheus_url,
        instance=args.prometheus_instance,
    )

    print(f"Running benchmark on {len(target_repos)} repos on node...")
    print(f"Storage: {storage_dir}, Results: {results_path}")

    with open(results_path, "a", encoding="utf-8") as out_f:
        for r in target_repos:
            print(f"\n---> Starting: {r['repo']} (sha: {r['commit'][:8]})")
            res = None
            for attempt in range(1, 4):
                res = run_benchmark_for_repo(
                    r,
                    storage_dir,
                    ridx_bin,
                    args.ra,
                    taskset_cpus=args.taskset,
                    sample_count=args.samples,
                    w1_steps=args.w1_steps,
                    w2_iterations=args.w2_iterations,
                    w3_counts=tuple(args.w3_counts),
                    discrepancies_path=discrepancies_path,
                    prom_collector=prom_collector,
                )
                if res.get("status") != "NOISE_REJECTED" or attempt == 3:
                    break
                print(f"    [Noise Rejected: {res.get('noise_reason')}] Waiting 15s before retry {attempt + 1}/3...")
                time.sleep(15)
            print(f"---> Finished {r['repo']}: status={res.get('status')}")
            out_f.write(json.dumps(res) + "\n")
            out_f.flush()

    print("\nBenchmark pass completed!")


if __name__ == "__main__":
    main()
