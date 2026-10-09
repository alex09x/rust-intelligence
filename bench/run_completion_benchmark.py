# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Targeted completion benchmark runner for RIDX vs rust-analyzer.
Measures warm/cold latency breakdown for owner-path vs fallback,
computes recall against rust-analyzer, and exports published jsonl.
"""

import json
import os
import random
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

bench_dir = Path(__file__).resolve().parent
if str(bench_dir) not in sys.path:
    sys.path.insert(0, str(bench_dir))

from lsp_process import SimpleLspProcess
from metrics import percentile
from scan import scan_source_positions


def sample_completion_scenarios(repo_dir, sample_count=100, seed=42):
    """Samples balanced dot and colon completion positions from real code."""
    random.seed(seed)
    repo_path = Path(repo_dir)
    if (repo_path / "crates" / "bevy_ecs").exists():
        scan_targets = [repo_path / "crates" / "bevy_ecs"]
    elif (repo_path / "serde").exists() and (repo_path / "serde_core").exists():
        scan_targets = [repo_path / "serde", repo_path / "serde_core"]
    else:
        scan_targets = [repo_path]

    usages = []
    for st in scan_targets:
        u, _ = scan_source_positions(str(st))
        usages.extend(u)

    # Filter out build scripts and non-src files
    valid_usages = [
        u for u in usages
        if "build.rs" not in u[0] and "src/" in u[0]
    ]

    dots = [u for u in valid_usages if u[4] == "method_call"]
    colons = [u for u in valid_usages if u[4] == "path_ident"]

    half = sample_count // 2
    sampled_dots = random.sample(dots, min(half, len(dots))) if dots else []
    sampled_colons = random.sample(colons, min(sample_count - len(sampled_dots), len(colons))) if colons else []

    scenarios = []
    for i, (f, line, col, name, _) in enumerate(sampled_dots):
        scenarios.append({
            "name": f"comp_dot_{i}_{name}",
            "kind": "dot",
            "target": name,
            "location": [f, line, col],
            "file": f,
            "line": line,
            "col": col,
        })
    for i, (f, line, col, name, _) in enumerate(sampled_colons):
        scenarios.append({
            "name": f"comp_colon_{i}_{name}",
            "kind": "colon",
            "target": name,
            "location": [f, line, col],
            "file": f,
            "line": line,
            "col": col,
        })

    random.shuffle(scenarios)
    return scenarios[:sample_count]


def eval_completion_repo(repo_slug, repo_dir, ridx_bin, commit_ridx, seed=42, sample_count=100, ra_bin=None):
    """Runs symmetric completion benchmark on a single repository."""
    scenarios = sample_completion_scenarios(repo_dir, sample_count, seed)
    print(f"[{repo_slug}] Running {len(scenarios)} completion scenarios (50/50 dot/colon)...")

    if not ra_bin:
        ra_bin_custom = Path.home() / ".rustup" / "toolchains" / "1.98.1-x86_64-unknown-linux-gnu" / "bin" / "rust-analyzer"
        ra_bin = str(ra_bin_custom) if ra_bin_custom.is_file() else "rust-analyzer"
    ra_ver = subprocess.run([ra_bin, "--version"], capture_output=True, text=True, check=False).stdout.strip()
    ridx_srv = SimpleLspProcess([str(ridx_bin), "lsp"], repo_dir)
    ra_srv = SimpleLspProcess([ra_bin], repo_dir, is_ra=True)

    # Pre-open buffers for warm scenario execution
    opened_files = set()
    for sc in scenarios:
        f = sc["file"]
        if f not in opened_files and Path(f).is_file():
            opened_files.add(f)
            try:
                with open(f, "r", encoding="utf-8", errors="replace") as fp:
                    content = fp.read()
                did_open = {
                    "textDocument": {"uri": f"file://{f}", "languageId": "rust", "version": 1, "text": content}
                }
                ridx_srv.notify("textDocument/didOpen", did_open)
                ra_srv.notify("textDocument/didOpen", did_open)
            except (OSError, UnicodeDecodeError):
                continue

    if ra_srv.is_ra:
        ra_srv.wait_quiescent(timeout=10.0, settle_idle=0.5)

    results = []
    owner_warm_latencies = []
    owner_cold_latencies = []
    fallback_warm_latencies = []
    fallback_cold_latencies = []
    owner_ra_warm_latencies = []
    fallback_ra_warm_latencies = []
    ra_warm_latencies = []
    ra_cold_latencies = []
    recalls = []
    precisions = []

    for idx, sc in enumerate(scenarios):
        f, line, col = sc["file"], sc["line"], sc["col"]
        params = {
            "textDocument": {"uri": f"file://{f}"},
            "position": {"line": line - 1, "character": col - 1},
        }

        # 3 iterations for RIDX
        ridx_durs = []
        ridx_res = None
        for it in range(3):
            res, dur, _ = ridx_srv.request("textDocument/completion", params, timeout=5.0)
            ridx_durs.append(dur)
            if it == 0:
                ridx_res = res

        # 3 iterations for rust-analyzer
        ra_durs = []
        ra_res = None
        for it in range(3):
            res, dur, _ = ra_srv.request("textDocument/completion", params, timeout=10.0)
            ra_durs.append(dur)
            if it == 0:
                ra_res = res

        ridx_cold = ridx_durs[0]
        ridx_warm = sorted(ridx_durs[1:])[len(ridx_durs[1:]) // 2] if len(ridx_durs) > 1 else ridx_cold

        ra_cold = ra_durs[0]
        ra_warm = sorted(ra_durs[1:])[len(ra_durs[1:]) // 2] if len(ra_durs) > 1 else ra_cold

        # Route detection: owner-path produces detail with "::" and " (in "
        ridx_items = ridx_res.get("items", []) if ridx_res else []
        ra_items = ra_res.get("items", []) if ra_res else []

        is_owner = False
        if ridx_items:
            first_det = ridx_items[0].get("detail", "")
            if " (in " in first_det:
                owner_part = first_det.rsplit(" (in ", 1)[1].rstrip(")")
                is_owner = "::" in owner_part

        route = "owner_path" if is_owner else "fallback"
        if is_owner:
            owner_warm_latencies.append(ridx_warm)
            owner_cold_latencies.append(ridx_cold)
            owner_ra_warm_latencies.append(ra_warm)
        else:
            fallback_warm_latencies.append(ridx_warm)
            fallback_cold_latencies.append(ridx_cold)
            fallback_ra_warm_latencies.append(ra_warm)

        ra_warm_latencies.append(ra_warm)
        ra_cold_latencies.append(ra_cold)

        # Recall and precision computation vs rust-analyzer
        labels_ridx = {it.get("label", "").split("(")[0].strip() for it in ridx_items}
        labels_ra = {it.get("label", "").split("(")[0].strip() for it in ra_items}
        matched = len(labels_ridx & labels_ra)
        rec = (matched / len(labels_ra) * 100.0) if labels_ra else (100.0 if not labels_ridx else 0.0)
        recalls.append(rec)
        prec = (matched / len(labels_ridx) * 100.0) if labels_ridx else 100.0
        precisions.append(prec)

        results.append({
            "name": sc["name"],
            "target": sc["target"],
            "kind": sc["kind"],
            "location": [str(Path(f).relative_to(repo_dir)), line, col],
            "route": route,
            "ridx_warm_us": round(ridx_warm),
            "ridx_cold_us": round(ridx_cold),
            "ra_warm_us": round(ra_warm),
            "ra_cold_us": round(ra_cold),
            "recall_pct": round(rec, 1),
            "precision_pct": round(prec, 1),
            "ridx_count": len(ridx_items),
            "ra_count": len(ra_items),
        })

        if (idx + 1) % 25 == 0:
            print(f"  Processed {idx + 1}/{len(scenarios)}: latest route={route}, ridx={ridx_warm:.0f}µs, ra={ra_warm:.0f}µs")

    ridx_srv.notify("exit", {})
    ra_srv.notify("exit", {})

    total_eval = len(scenarios)
    fallback_cnt = len(fallback_warm_latencies)
    fallback_pct = (fallback_cnt / total_eval * 100.0) if total_eval else 0.0

    all_ridx_warm = owner_warm_latencies + fallback_warm_latencies
    all_ridx_cold = owner_cold_latencies + fallback_cold_latencies

    owner_ra_warm_p50 = percentile(owner_ra_warm_latencies, 50.0) if owner_ra_warm_latencies else 0.0
    fallback_ra_warm_p50 = percentile(fallback_ra_warm_latencies, 50.0) if fallback_ra_warm_latencies else 0.0

    metrics = {
        "ridx_completion_warm_p50_us": round(percentile(all_ridx_warm, 50.0), 1),
        "ridx_completion_warm_p95_us": round(percentile(all_ridx_warm, 95.0), 1),
        "ridx_completion_cold_p50_us": round(percentile(all_ridx_cold, 50.0), 1),
        "ridx_completion_cold_p95_us": round(percentile(all_ridx_cold, 95.0), 1),
        "ridx_owner_warm_p50_us": round(percentile(owner_warm_latencies, 50.0), 1) if owner_warm_latencies else 0.0,
        "ridx_owner_warm_p95_us": round(percentile(owner_warm_latencies, 95.0), 1) if owner_warm_latencies else 0.0,
        "ridx_owner_cold_p50_us": round(percentile(owner_cold_latencies, 50.0), 1) if owner_cold_latencies else 0.0,
        "ridx_owner_cold_p95_us": round(percentile(owner_cold_latencies, 95.0), 1) if owner_cold_latencies else 0.0,
        "ridx_fallback_warm_p50_us": round(percentile(fallback_warm_latencies, 50.0), 1) if fallback_warm_latencies else 0.0,
        "ridx_fallback_warm_p95_us": round(percentile(fallback_warm_latencies, 95.0), 1) if fallback_warm_latencies else 0.0,
        "ridx_fallback_cold_p50_us": round(percentile(fallback_cold_latencies, 50.0), 1) if fallback_cold_latencies else 0.0,
        "ridx_fallback_cold_p95_us": round(percentile(fallback_cold_latencies, 95.0), 1) if fallback_cold_latencies else 0.0,
        "fallback_share_pct": round(fallback_pct, 1),
        "owner_share_pct": round(100.0 - fallback_pct, 1),
        "ra_completion_warm_p50_us": round(percentile(ra_warm_latencies, 50.0), 1),
        "ra_completion_warm_p95_us": round(percentile(ra_warm_latencies, 95.0), 1),
        "ra_matching_owner_warm_p50_us": round(owner_ra_warm_p50, 1),
        "ra_matching_fallback_warm_p50_us": round(fallback_ra_warm_p50, 1),
        "ra_completion_cold_p50_us": round(percentile(ra_cold_latencies, 50.0), 1),
        "ra_completion_cold_p95_us": round(percentile(ra_cold_latencies, 95.0), 1),
        "avg_recall_vs_ra_pct": round(sum(recalls) / len(recalls), 1) if recalls else 0.0,
        "avg_precision_vs_ra_pct": round(sum(precisions) / len(precisions), 1) if precisions else 100.0,
        "warm_p50_speedup_vs_ra": round(percentile(ra_warm_latencies, 50.0) / max(1.0, percentile(all_ridx_warm, 50.0)), 2),
        "owner_p50_speedup_vs_ra": round(owner_ra_warm_p50 / max(1.0, percentile(owner_warm_latencies, 50.0)), 2) if owner_warm_latencies else 0.0,
        "fallback_p50_speedup_vs_ra": round(fallback_ra_warm_p50 / max(1.0, percentile(fallback_warm_latencies, 50.0)), 2) if fallback_warm_latencies else 0.0,
    }

    return {
        "repo": repo_slug,
        "commit_ridx": commit_ridx,
        "baseline_ra": ra_ver,
        "node": "booster (AMD Ryzen 9 7950X, 16 cores / 32 threads, 128 GB RAM)",
        "taskset": "24-31",
        "seed": seed,
        "timestamp": datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z"),
        "sample_count": len(scenarios),
        "capability": "completion",
        "metrics": metrics,
        "scenarios": results,
        "measurement_note": "symmetric live server session; first_query (cold) = iter 0 after didOpen scenario buffer warmup; warm = median(iter 1..2)"
    }


def main():
    import argparse
    parser = argparse.ArgumentParser(description="Run completion benchmarks")
    parser.add_argument("--commit", default="8fa53fa")
    parser.add_argument("--ridx-bin", help="Path to ridx binary")
    parser.add_argument("--ra-bin", help="Path to rust-analyzer binary")
    parser.add_argument("--serde-dir", help="Path to serde repo")
    parser.add_argument("--bevy-dir", help="Path to bevy repo")
    args = parser.parse_args()

    commit = args.commit
    ridx_bin = Path(args.ridx_bin) if args.ridx_bin else (bench_dir.parent / "target" / "release" / "ridx")
    out_file = bench_dir / "published" / f"rerun_completion_{commit}.jsonl"

    def default_repo(name):
        env_key = f"BENCH_{name.upper()}_DIR"
        if env_key in os.environ:
            return Path(os.environ[env_key])
        for cand in [
            Path.home() / "prod-code-storage" / "bench_repos" / name,
            bench_dir.parent.parent / "bench_repos" / name,
            Path("/tmp") / name,
        ]:
            if cand.is_dir():
                return cand
        return Path("/tmp") / name

    serde_path = Path(args.serde_dir) if args.serde_dir else default_repo("serde")
    bevy_path = Path(args.bevy_dir) if args.bevy_dir else default_repo("bevy")

    repos = [
        ("serde-rs/serde", str(serde_path)),
        ("bevyengine/bevy", str(bevy_path)),
    ]

    out_file.parent.mkdir(parents=True, exist_ok=True)
    with open(out_file, "w", encoding="utf-8") as f_out:
        for slug, rpath in repos:
            data = eval_completion_repo(slug, rpath, ridx_bin, commit, seed=42, sample_count=100, ra_bin=args.ra_bin)
            f_out.write(json.dumps(data) + "\n")
            f_out.flush()
            m = data["metrics"]
            print(f"\n--- {slug} Summary ---")
            print(f"  Owner-Path Latency:  p50 = {m['ridx_owner_warm_p50_us']} µs, p95 = {m['ridx_owner_warm_p95_us']} µs")
            print(f"  Fallback Latency:    p50 = {m['ridx_fallback_warm_p50_us']} µs, p95 = {m['ridx_fallback_warm_p95_us']} µs")
            print(f"  Fallback Share:      {m['fallback_share_pct']}% (Owner: {m['owner_share_pct']}%)")
            print(f"  Overall RIDX Warm:   p50 = {m['ridx_completion_warm_p50_us']} µs, p95 = {m['ridx_completion_warm_p95_us']} µs")
            print(f"  RA Warm:             p50 = {m['ra_completion_warm_p50_us']} µs, p95 = {m['ra_completion_warm_p95_us']} µs")
            print(f"  Speedup vs RA:       {m['warm_p50_speedup_vs_ra']}x (Owner: {m['owner_p50_speedup_vs_ra']}x)")
            print(f"  Recall vs RA:        {m['avg_recall_vs_ra_pct']}%")
            print(f"  Precision vs RA:     {m.get('avg_precision_vs_ra_pct', 0.0)}%")

    print(f"\nSuccessfully generated and published: {out_file}")


if __name__ == "__main__":
    main()
