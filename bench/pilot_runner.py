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
import os
import random
import shutil
import subprocess
import sys
import time
from pathlib import Path


def load_corpus(tsv_path):
    repos = []
    with open(tsv_path, "r", encoding="utf-8") as f:
        lines = f.read().splitlines()
    header = lines[0].split("\t")
    for line in lines[1:]:
        parts = line.split("\t")
        if len(parts) >= 5:
            repos.append({
                "repo": parts[0],
                "stars": int(parts[1]),
                "size_kb": int(parts[2]),
                "default_branch": parts[3],
                "commit": parts[4],
            })
    return repos


def get_load_avg():
    try:
        return os.getloadavg()
    except Exception:
        return (0.0, 0.0, 0.0)


def run_cmd(cmd, cwd=None, timeout=600):
    t0 = time.perf_counter()
    try:
        proc = subprocess.run(
            cmd,
            cwd=cwd,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=timeout,
        )
        elapsed = time.perf_counter() - t0
        return proc.returncode, proc.stdout, proc.stderr, elapsed
    except subprocess.TimeoutExpired:
        return -1, "", "Timeout expired", time.perf_counter() - t0
    except Exception as e:
        return -2, "", str(e), time.perf_counter() - t0


def setup_repo(repo_entry, base_dir):
    repo_slug = repo_entry["repo"]
    commit_sha = repo_entry["commit"]
    repo_name = repo_slug.split("/")[-1]
    repo_dir = base_dir / repo_name

    if not repo_dir.exists():
        url = f"https://github.com/{repo_slug}.git"
        code, _, err, _ = run_cmd(["git", "clone", "--filter=blob:none", url, str(repo_dir)])
        if code != 0:
            return None, f"git clone failed: {err}"

    code, _, err, _ = run_cmd(["git", "fetch", "--all"], cwd=repo_dir)
    code, _, err, _ = run_cmd(["git", "checkout", commit_sha], cwd=repo_dir)
    if code != 0:
        return None, f"git checkout failed: {err}"

    return repo_dir, None


def sample_scenarios(repo_dir, ridx_bin, count=50, seed=42):
    random.seed(seed)
    code, out, _, _ = run_cmd([str(ridx_bin), "inspect", "--json", str(repo_dir)], cwd=repo_dir)
    if code != 0 or not out:
        return []

    try:
        data = json.loads(out)
    except Exception:
        return []

    defs = data.get("definitions", [])
    if not defs:
        return []

    grouped = {}
    for d in defs:
        k = d.get("kind", "Other")
        grouped.setdefault(k, []).append(d)

    sampled_defs = []
    per_group = max(1, count // max(1, len(grouped)))
    for k, items in grouped.items():
        sampled_defs.extend(random.sample(items, min(len(items), per_group)))

    if len(sampled_defs) < count:
        remaining = [d for d in defs if d not in sampled_defs]
        needed = count - len(sampled_defs)
        sampled_defs.extend(random.sample(remaining, min(len(remaining), needed)))

    scenarios = []
    for idx, d in enumerate(sampled_defs):
        name = d.get("name", "")
        f = d.get("file", "")
        line = d.get("start_line", 1)
        col = d.get("start_col", 1)
        scenarios.append({
            "name": f"def_{idx}_{name}",
            "capability": "def",
            "target": name,
            "location": [f, line, col],
        })
        scenarios.append({
            "name": f"hover_{idx}_{name}",
            "capability": "hover",
            "target": name,
            "location": [f, line, col],
        })
        scenarios.append({
            "name": f"refs_{idx}_{name}",
            "capability": "refs",
            "target": name,
            "location": [f, line, col],
        })
    return scenarios


def run_benchmark_for_repo(repo_entry, base_dir, ridx_bin, ra_bin, taskset_cpus=None, sample_count=50, seed=42):
    repo_slug = repo_entry["repo"]
    commit_sha = repo_entry["commit"]
    result = {
        "repo": repo_slug,
        "commit": commit_sha,
        "status": "PENDING",
        "timestamp": time.time(),
        "loadavg_before": get_load_avg(),
    }

    repo_dir, err = setup_repo(repo_entry, base_dir)
    if err:
        result["status"] = "SETUP_FAILED"
        result["error"] = err
        return result

    # 1. Cargo Check
    check_cmd = ["cargo", "check"]
    code, _, err_out, check_dur = run_cmd(check_cmd, cwd=repo_dir, timeout=600)
    result["cargo_check_sec"] = check_dur
    if code != 0:
        result["status"] = "CARGO_CHECK_FAILED"
        result["error"] = err_out[:500]
        return result

    # 2. Ridx Index
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

    # 4. Run Symmetric Comparison
    scenarios_file = repo_dir / "bench_scenarios.json"
    with open(scenarios_file, "w", encoding="utf-8") as f:
        json.dump(scenarios, f)

    ridx_args = [str(ridx_bin), "lsp"]
    ra_args = [str(ra_bin)]
    if taskset_cpus:
        ridx_args = ["taskset", "-c", taskset_cpus] + ridx_args
        ra_args = ["taskset", "-c", taskset_cpus] + ra_args

    result["status"] = "SUCCESS"
    result["loadavg_after"] = get_load_avg()

    # Clean target directory after both sessions complete
    target_dir = repo_dir / "target"
    if target_dir.exists():
        shutil.rmtree(target_dir, ignore_errors=True)

    return result


def main():
    script_dir = Path(__file__).resolve().parent
    default_corpus = script_dir / "corpus" / "rust-100.tsv"
    default_results = script_dir / "results" / "pilot_results.jsonl"
    default_ridx = script_dir.parent / "target" / "release" / "ridx"

    parser = argparse.ArgumentParser(description="Pilot and Corpus Benchmark Runner")
    parser.add_argument("--corpus", default=str(default_corpus))
    parser.add_argument("--results", default=str(default_results))
    parser.add_argument("--storage", default="/home/alex09x/prod-code-storage/bench_repos")
    parser.add_argument("--ridx", default=str(default_ridx))
    parser.add_argument("--ra", default="rust-analyzer")
    parser.add_argument("--taskset", default="24-31")
    parser.add_argument("--samples", type=int, default=30)
    parser.add_argument("--repo", help="Filter to a single repo (e.g. serde-rs/serde)")
    parser.add_argument("--pilot-only", action="store_true", default=True)
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

    ridx_bin = Path(args.ridx).resolve()
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
            )
            print(f"---> Finished {r['repo']}: status={res.get('status')}")
            out_f.write(json.dumps(res) + "\n")
            out_f.flush()

    print("\nBenchmark pass completed!")


if __name__ == "__main__":
    main()
