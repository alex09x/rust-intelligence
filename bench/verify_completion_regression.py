# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Empirical regression and parity verification: bb4aa75 vs 8fa53fa vs rust-analyzer 1.98.1.
Compares candidate counts, empty response rates, recall, and fair matching-subset speedups.
"""

import json
import os
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

bench_dir = Path(__file__).resolve().parent
if str(bench_dir) not in sys.path:
    sys.path.insert(0, str(bench_dir))

from lsp_process import SimpleLspProcess
from metrics import percentile


def load_canonical_scenarios(jsonl_path):
    """Loads exact scenarios from canonical rerun_completion_8fa53fa.jsonl."""
    repos_data = {}
    with open(jsonl_path, "r", encoding="utf-8") as f:
        for line in f:
            if not line.strip():
                continue
            entry = json.loads(line)
            repos_data[entry["repo"]] = entry["scenarios"]
    return repos_data


def preopen_scenario_buffers(srv, repo_dir, scenarios):
    """Pre-opens document buffers so server caches AST / line index."""
    opened = set()
    for sc in scenarios:
        loc = sc["location"]
        rel_path = loc[0]
        full_path = Path(repo_dir) / rel_path
        if full_path not in opened and full_path.is_file():
            opened.add(full_path)
            try:
                with open(full_path, "r", encoding="utf-8", errors="replace") as fp:
                    content = fp.read()
                srv.notify("textDocument/didOpen", {
                    "textDocument": {
                        "uri": f"file://{full_path}",
                        "languageId": "rust",
                        "version": 1,
                        "text": content,
                    }
                })
            except (OSError, UnicodeDecodeError):
                pass


def run_scenarios_on_server(srv, repo_dir, scenarios, is_ra=False, timeout=5.0):
    """Executes completion requests for 3 iterations per scenario."""
    if is_ra and hasattr(srv, "wait_quiescent"):
        srv.wait_quiescent(timeout=10.0, settle_idle=0.5)

    results = []
    for idx, sc in enumerate(scenarios):
        loc = sc["location"]
        rel_path, line, col = loc[0], loc[1], loc[2]
        full_path = Path(repo_dir) / rel_path
        params = {
            "textDocument": {"uri": f"file://{full_path}"},
            "position": {"line": line - 1, "character": col - 1},
        }

        durs = []
        first_res = None
        for it in range(3):
            res, dur, _ = srv.request("textDocument/completion", params, timeout=timeout)
            durs.append(dur)
            if it == 0:
                first_res = res

        cold = durs[0]
        warm = sorted(durs[1:])[len(durs[1:]) // 2] if len(durs) > 1 else cold
        items = first_res.get("items", []) if first_res else []

        results.append({
            "name": sc["name"],
            "location": loc,
            "cold_us": cold,
            "warm_us": warm,
            "count": len(items),
            "labels": [it.get("label", "").split("(")[0].strip() for it in items],
            "first_detail": items[0].get("detail", "") if items else "",
        })
        if (idx + 1) % 25 == 0:
            print(f"    Completed {idx + 1}/{len(scenarios)} scenarios...")

    return results


def find_default_ra():
    ra_custom = Path.home() / ".rustup" / "toolchains" / "1.98.1-x86_64-unknown-linux-gnu" / "bin" / "rust-analyzer"
    if ra_custom.is_file():
        return str(ra_custom)
    return "rust-analyzer"


def find_default_repo(name):
    env_key = f"BENCH_{name.upper()}_DIR"
    if env_key in os.environ:
        return Path(os.environ[env_key])
    candidates = [
        Path.home() / "prod-code-storage" / "bench_repos" / name,
        bench_dir.parent.parent / "bench_repos" / name,
        Path("/tmp") / name,
    ]
    for c in candidates:
        if c.is_dir():
            return c
    return candidates[0]


def run_verification(args=None):
    """Runs complete 3-way evaluation on serde and bevy."""
    published_jsonl = bench_dir / "published" / "rerun_completion_8fa53fa.jsonl"
    canonical_data = load_canonical_scenarios(published_jsonl)

    bb4aa75_bin = Path(args.bb_bin) if args and args.bb_bin else Path("/tmp/ridx-bb4aa75/target/release/ridx")
    curr_bin = Path(args.ridx_bin) if args and args.ridx_bin else (bench_dir.parent / "target" / "release" / "ridx")
    ra_bin = args.ra_bin if args and args.ra_bin else find_default_ra()

    serde_dir = Path(args.serde_dir) if args and args.serde_dir else find_default_repo("serde")
    bevy_dir = Path(args.bevy_dir) if args and args.bevy_dir else find_default_repo("bevy")

    ra_ver = subprocess.run([ra_bin, "--version"], capture_output=True, text=True, check=False).stdout.strip()
    print(f"=== Rust-Analyzer Baseline: {ra_ver} ===")

    repos = [
        ("serde-rs/serde", str(serde_dir), "serde"),
        ("bevyengine/bevy", str(bevy_dir), "bevy_ecs"),
    ]

    all_comparisons = {}
    updated_published_records = []

    for slug, rpath, pkg in repos:
        scenarios = canonical_data[slug]
        print(f"\n[{slug}] Evaluating {len(scenarios)} scenarios across bb4aa75, 8fa53fa, and RA 1.98.1...")

        # --- Phase 1: bb4aa75 baseline ---
        print(f"  [1/3] Indexing and running bb4aa75 baseline on {slug}...")
        subprocess.run([str(bb4aa75_bin), "index", "-p", pkg, rpath], check=True, capture_output=True)
        srv_bb = SimpleLspProcess([str(bb4aa75_bin), "lsp"], rpath)
        preopen_scenario_buffers(srv_bb, rpath, scenarios)
        res_bb = run_scenarios_on_server(srv_bb, rpath, scenarios, is_ra=False, timeout=5.0)
        srv_bb.notify("exit", {})

        # --- Phase 2: current commit 8fa53fa ---
        print(f"  [2/3] Indexing and running 8fa53fa on {slug}...")
        subprocess.run([str(curr_bin), "index", "-p", pkg, rpath], check=True, capture_output=True)
        srv_curr = SimpleLspProcess([str(curr_bin), "lsp"], rpath)
        preopen_scenario_buffers(srv_curr, rpath, scenarios)
        res_curr = run_scenarios_on_server(srv_curr, rpath, scenarios, is_ra=False, timeout=5.0)
        srv_curr.notify("exit", {})

        # --- Phase 3: rust-analyzer 1.98.1 ---
        print(f"  [3/3] Running rust-analyzer 1.98.1 on {slug}...")
        srv_ra = SimpleLspProcess([ra_bin], rpath, is_ra=True)
        preopen_scenario_buffers(srv_ra, rpath, scenarios)
        res_ra = run_scenarios_on_server(srv_ra, rpath, scenarios, is_ra=True, timeout=10.0)
        srv_ra.notify("exit", {})

        # --- Comparison & Analysis ---
        empty_bb = 0
        empty_curr = 0
        empty_ra = 0
        total_items_bb = 0
        total_items_curr = 0
        total_items_ra = 0
        regressions_count = 0
        improvements_count = 0

        owner_scenarios = []
        fallback_scenarios = []
        sc_published = []

        recalls_curr = []
        recalls_bb = []

        for idx, sc in enumerate(scenarios):
            b = res_bb[idx]
            c = res_curr[idx]
            r = res_ra[idx]

            if b["count"] == 0:
                empty_bb += 1
            if c["count"] == 0:
                empty_curr += 1
            if r["count"] == 0:
                empty_ra += 1

            total_items_bb += b["count"]
            total_items_curr += c["count"]
            total_items_ra += r["count"]

            if c["count"] < b["count"]:
                regressions_count += 1
            elif c["count"] > b["count"]:
                improvements_count += 1

            # Route detection in 8fa53fa
            is_owner = False
            first_det = c["first_detail"]
            if " (in " in first_det:
                owner_part = first_det.rsplit(" (in ", 1)[1].rstrip(")")
                is_owner = "::" in owner_part

            route = "owner_path" if is_owner else "fallback"

            labels_c = set(c["labels"])
            labels_b = set(b["labels"])
            labels_r = set(r["labels"])

            matched_c = len(labels_c & labels_r)
            rec_c = (matched_c / len(labels_r) * 100.0) if labels_r else (100.0 if not labels_c else 0.0)
            recalls_curr.append(rec_c)

            matched_b = len(labels_b & labels_r)
            rec_b = (matched_b / len(labels_r) * 100.0) if labels_r else (100.0 if not labels_b else 0.0)
            recalls_bb.append(rec_b)

            entry_pub = {
                "name": sc["name"],
                "target": sc["target"],
                "kind": sc["kind"],
                "location": sc["location"],
                "route": route,
                "ridx_warm_us": round(c["warm_us"]),
                "ridx_cold_us": round(c["cold_us"]),
                "ra_warm_us": round(r["warm_us"]),
                "ra_cold_us": round(r["cold_us"]),
                "recall_pct": round(rec_c, 1),
                "ridx_count": c["count"],
                "ra_count": r["count"],
                "bb4aa75_count": b["count"],
                "bb4aa75_recall_pct": round(rec_b, 1),
            }
            sc_published.append(entry_pub)

            if is_owner:
                owner_scenarios.append(entry_pub)
            else:
                fallback_scenarios.append(entry_pub)

        owner_c_warm = [s["ridx_warm_us"] for s in owner_scenarios]
        owner_ra_warm = [s["ra_warm_us"] for s in owner_scenarios]
        fallback_c_warm = [s["ridx_warm_us"] for s in fallback_scenarios]
        fallback_ra_warm = [s["ra_warm_us"] for s in fallback_scenarios]

        owner_c_p50 = percentile(owner_c_warm, 50.0) if owner_c_warm else 0.0
        owner_ra_p50 = percentile(owner_ra_warm, 50.0) if owner_ra_warm else 0.0
        fallback_c_p50 = percentile(fallback_c_warm, 50.0) if fallback_c_warm else 0.0
        fallback_ra_p50 = percentile(fallback_ra_warm, 50.0) if fallback_ra_warm else 0.0

        all_c_warm = [s["ridx_warm_us"] for s in sc_published]
        all_ra_warm = [s["ra_warm_us"] for s in sc_published]
        all_c_p50 = percentile(all_c_warm, 50.0)
        all_ra_p50 = percentile(all_ra_warm, 50.0)

        owner_speedup = round(owner_ra_p50 / max(1.0, owner_c_p50), 2) if owner_c_p50 > 0 else 0.0
        fallback_speedup = round(fallback_ra_p50 / max(1.0, fallback_c_p50), 2) if fallback_c_p50 > 0 else 0.0
        overall_speedup = round(all_ra_p50 / max(1.0, all_c_p50), 2)

        comparison_summary = {
            "repo": slug,
            "total_scenarios": len(scenarios),
            "empty_responses": {
                "bb4aa75": empty_bb,
                "8fa53fa": empty_curr,
                "rust_analyzer_1_98_1": empty_ra,
            },
            "total_candidates": {
                "bb4aa75": total_items_bb,
                "8fa53fa": total_items_curr,
                "rust_analyzer_1_98_1": total_items_ra,
            },
            "avg_recall_vs_ra": {
                "bb4aa75": round(sum(recalls_bb) / len(recalls_bb), 1),
                "8fa53fa": round(sum(recalls_curr) / len(recalls_curr), 1),
            },
            "regressions_scenarios": regressions_count,
            "improvements_scenarios": improvements_count,
            "routes": {
                "owner_count": len(owner_scenarios),
                "fallback_count": len(fallback_scenarios),
                "owner_c_warm_p50_us": round(owner_c_p50, 1),
                "owner_ra_warm_p50_us": round(owner_ra_p50, 1),
                "owner_speedup_fair": owner_speedup,
                "fallback_c_warm_p50_us": round(fallback_c_p50, 1),
                "fallback_ra_warm_p50_us": round(fallback_ra_p50, 1),
                "fallback_speedup_fair": fallback_speedup,
                "overall_c_warm_p50_us": round(all_c_p50, 1),
                "overall_ra_warm_p50_us": round(all_ra_p50, 1),
                "overall_speedup": overall_speedup,
            }
        }
        all_comparisons[slug] = comparison_summary

        print(f"\n--- Results for {slug} ---")
        print(f"  Empty responses:      bb4aa75: {empty_bb}, 8fa53fa: {empty_curr}, RA: {empty_ra}")
        print(f"  Total items returned: bb4aa75: {total_items_bb}, 8fa53fa: {total_items_curr}, RA: {total_items_ra}")
        print(f"  Avg recall vs RA:     bb4aa75: {comparison_summary['avg_recall_vs_ra']['bb4aa75']}%, 8fa53fa: {comparison_summary['avg_recall_vs_ra']['8fa53fa']}%")
        print(f"  Scenario diffs:       {regressions_count} regressions, {improvements_count} improvements")
        print(f"  Fair Owner Speedup:   {owner_speedup}x ({owner_ra_p50:.0f}µs RA vs {owner_c_p50:.0f}µs 8fa53fa on {len(owner_scenarios)} matching positions)")
        print(f"  Fair Fallback Speedup:{fallback_speedup}x ({fallback_ra_p50:.0f}µs RA vs {fallback_c_p50:.0f}µs 8fa53fa on {len(fallback_scenarios)} matching positions)")

        # Prepare updated published format
        updated_published_records.append({
            "repo": slug,
            "commit_ridx": "8fa53fa",
            "baseline_ra": ra_ver,
            "node": "booster (AMD Ryzen 9 7950X, 16 cores / 32 threads, 128 GB RAM)",
            "taskset": "24-31",
            "seed": 42,
            "timestamp": datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z"),
            "sample_count": len(scenarios),
            "capability": "completion",
            "metrics": {
                "ridx_completion_warm_p50_us": round(all_c_p50, 1),
                "ridx_completion_warm_p95_us": round(percentile(all_c_warm, 95.0), 1),
                "ridx_owner_warm_p50_us": round(owner_c_p50, 1),
                "ridx_owner_warm_p95_us": round(percentile([s["ridx_warm_us"] for s in owner_scenarios], 95.0), 1) if owner_scenarios else 0.0,
                "ridx_fallback_warm_p50_us": round(fallback_c_p50, 1),
                "ridx_fallback_warm_p95_us": round(percentile([s["ridx_warm_us"] for s in fallback_scenarios], 95.0), 1) if fallback_scenarios else 0.0,
                "ra_matching_owner_warm_p50_us": round(owner_ra_p50, 1),
                "ra_matching_fallback_warm_p50_us": round(fallback_ra_p50, 1),
                "ra_overall_warm_p50_us": round(all_ra_p50, 1),
                "owner_p50_speedup_vs_ra": owner_speedup,
                "fallback_p50_speedup_vs_ra": fallback_speedup,
                "overall_p50_speedup_vs_ra": overall_speedup,
                "owner_share_pct": round(len(owner_scenarios) / len(scenarios) * 100.0, 1),
                "fallback_share_pct": round(len(fallback_scenarios) / len(scenarios) * 100.0, 1),
                "avg_recall_vs_ra_pct": comparison_summary["avg_recall_vs_ra"]["8fa53fa"],
                "bb4aa75_avg_recall_vs_ra_pct": comparison_summary["avg_recall_vs_ra"]["bb4aa75"],
                "empty_responses_ridx": empty_curr,
                "empty_responses_bb4aa75": empty_bb,
                "empty_responses_ra": empty_ra,
            },
            "scenarios": sc_published,
        })

    # Save summary report & updated published jsonl
    report_file = bench_dir / "published" / "completion_regression_bb4aa75_vs_8fa53fa.json"
    with open(report_file, "w", encoding="utf-8") as f:
        json.dump(all_comparisons, f, indent=2)

    with open(published_jsonl, "w", encoding="utf-8") as f:
        for rec in updated_published_records:
            f.write(json.dumps(rec) + "\n")

    print(f"\nVerification finished successfully!")
    print(f"Summary saved: {report_file}")
    print(f"Published rerun updated: {published_jsonl}")


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser(description="Verify completion regression")
    parser.add_argument("--bb-bin", help="Path to bb4aa75 ridx binary")
    parser.add_argument("--ridx-bin", help="Path to current ridx binary")
    parser.add_argument("--ra-bin", help="Path to rust-analyzer binary")
    parser.add_argument("--serde-dir", help="Path to serde repository")
    parser.add_argument("--bevy-dir", help="Path to bevy repository")
    args = parser.parse_args()
    run_verification(args)

