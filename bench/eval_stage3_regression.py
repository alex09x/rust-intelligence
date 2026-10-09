# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Stage 3 regression evaluation: 26a4ddb vs b05240a vs rust-analyzer 1.98.1.
Evaluates 100+100 canonical positions for generic trait bound resolution.
"""

import argparse
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


def load_baseline(baseline_file):
    """Loads baseline records from published jsonl."""
    records = {}
    with open(baseline_file, "r", encoding="utf-8") as f:
        for line in f:
            if not line.strip():
                continue
            d = json.loads(line)
            records[d["repo"]] = d
    return records


def preopen_buffers(srv, repo_dir, scenarios):
    """Pre-opens files in the LSP server."""
    opened = set()
    for sc in scenarios:
        rel_path = sc["location"][0]
        full_path = Path(repo_dir) / rel_path
        if full_path not in opened and full_path.is_file():
            opened.add(full_path)
            try:
                with open(full_path, "r", encoding="utf-8", errors="replace") as fp:
                    srv.notify("textDocument/didOpen", {
                        "textDocument": {
                            "uri": f"file://{full_path}",
                            "languageId": "rust",
                            "version": 1,
                            "text": fp.read(),
                        }
                    })
            except (OSError, UnicodeDecodeError):
                pass


def run_scenarios(srv, repo_dir, scenarios, is_ra=False, timeout=5.0):
    """Runs 3 iterations of completion for each scenario."""
    if is_ra and hasattr(srv, "wait_quiescent"):
        srv.wait_quiescent(timeout=10.0, settle_idle=0.5)

    results = []
    for idx, sc in enumerate(scenarios):
        rel_path, line, col = sc["location"]
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
            "location": sc["location"],
            "cold_us": cold,
            "warm_us": warm,
            "count": len(items),
            "labels": [it.get("label", "").split("(")[0].strip() for it in items],
            "first_detail": items[0].get("detail", "") if items else "",
        })
        if (idx + 1) % 25 == 0:
            print(f"    Completed {idx + 1}/{len(scenarios)} scenarios...")

    return results


def find_default_repo(name):
    env_key = f"BENCH_{name.upper()}_DIR"
    if env_key in os.environ:
        return Path(os.environ[env_key])
    for c in [
        Path.home() / "prod-code-storage" / "bench_repos" / name,
        bench_dir.parent.parent / "bench_repos" / name,
        Path("/tmp") / name,
    ]:
        if c.is_dir():
            return c
    return Path("/tmp") / name


def main():
    parser = argparse.ArgumentParser(description="Stage 3 regression evaluation")
    parser.add_argument("--base-commit", default="26a4ddb")
    parser.add_argument("--curr-commit", default="b05240a")
    parser.add_argument("--ridx-bin", help="Path to current ridx binary")
    parser.add_argument("--ra-bin", help="Path to rust-analyzer binary")
    parser.add_argument("--serde-dir", help="Path to serde repo")
    parser.add_argument("--bevy-dir", help="Path to bevy repo")
    args = parser.parse_args()

    base_jsonl = bench_dir / "published" / f"rerun_completion_{args.base_commit}.jsonl"
    base_data = load_baseline(base_jsonl)

    curr_bin = Path(args.ridx_bin) if args.ridx_bin else (bench_dir.parent / "target" / "release" / "ridx")
    ra_custom = Path.home() / ".rustup" / "toolchains" / "1.98.1-x86_64-unknown-linux-gnu" / "bin" / "rust-analyzer"
    ra_bin = args.ra_bin if args.ra_bin else (str(ra_custom) if ra_custom.is_file() else "rust-analyzer")
    ra_ver = subprocess.run([ra_bin, "--version"], capture_output=True, text=True, check=False).stdout.strip()

    serde_dir = Path(args.serde_dir) if args.serde_dir else find_default_repo("serde")
    bevy_dir = Path(args.bevy_dir) if args.bevy_dir else find_default_repo("bevy")

    repos = [
        ("serde-rs/serde", str(serde_dir)),
        ("bevyengine/bevy", str(bevy_dir)),
    ]

    all_comparisons = {}
    published_records = []

    for slug, rpath in repos:
        base_record = base_data[slug]
        scenarios = base_record["scenarios"]
        print(f"\n[{slug}] Evaluating {len(scenarios)} scenarios for {args.curr_commit} vs {args.base_commit}...")

        # Current RIDX server
        srv_curr = SimpleLspProcess([str(curr_bin), "lsp"], rpath)
        preopen_buffers(srv_curr, rpath, scenarios)
        res_curr = run_scenarios(srv_curr, rpath, scenarios, is_ra=False, timeout=5.0)
        srv_curr.notify("exit", {})

        # Rust-Analyzer server for exact candidate matching & recall
        srv_ra = SimpleLspProcess([ra_bin], rpath, is_ra=True)
        preopen_buffers(srv_ra, rpath, scenarios)
        res_ra = run_scenarios(srv_ra, rpath, scenarios, is_ra=True, timeout=10.0)
        srv_ra.notify("exit", {})

        empty_base = 0
        empty_curr = 0
        empty_ra = 0
        regressions_to_empty = 0
        closed_empty_count = 0
        closed_empty_names = []

        total_items_base = 0
        total_items_curr = 0
        total_items_ra = 0

        recalls_base = []
        recalls_curr = []

        owner_scenarios = []
        fallback_scenarios = []
        sc_published = []

        for idx, sc in enumerate(scenarios):
            c = res_curr[idx]
            r = res_ra[idx]
            base_cnt = sc["ridx_count"]

            if base_cnt == 0:
                empty_base += 1
            if c["count"] == 0:
                empty_curr += 1
            if r["count"] == 0:
                empty_ra += 1

            if base_cnt > 0 and c["count"] == 0:
                regressions_to_empty += 1
            if base_cnt == 0 and c["count"] > 0:
                closed_empty_count += 1
                closed_empty_names.append(sc["name"])

            total_items_base += base_cnt
            total_items_curr += c["count"]
            total_items_ra += r["count"]

            recalls_base.append(sc["recall_pct"])

            labels_c = set(c["labels"])
            labels_r = set(r["labels"])
            matched_c = len(labels_c & labels_r)
            rec_c = (matched_c / len(labels_r) * 100.0) if labels_r else (100.0 if not labels_c else 0.0)
            recalls_curr.append(rec_c)

            # Route detection: owner-path produces detail with " (in " and "::"
            is_owner = False
            first_det = c["first_detail"]
            if " (in " in first_det:
                owner_part = first_det.rsplit(" (in ", 1)[1].rstrip(")")
                is_owner = "::" in owner_part

            route = "owner_path" if is_owner else "fallback"

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
                "base_count": base_cnt,
                "base_recall_pct": sc["recall_pct"],
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

        avg_rec_base = round(sum(recalls_base) / len(recalls_base), 1)
        avg_rec_curr = round(sum(recalls_curr) / len(recalls_curr), 1)

        summary = {
            "repo": slug,
            "total_scenarios": len(scenarios),
            "empty_responses": {
                args.base_commit: empty_base,
                args.curr_commit: empty_curr,
                "rust_analyzer_1_98_1": empty_ra,
            },
            "closed_empty_responses": {
                "count": closed_empty_count,
                "scenarios": closed_empty_names,
            },
            "regressions_to_empty": regressions_to_empty,
            "total_candidates": {
                args.base_commit: total_items_base,
                args.curr_commit: total_items_curr,
                "rust_analyzer_1_98_1": total_items_ra,
            },
            "avg_recall_vs_ra": {
                args.base_commit: avg_rec_base,
                args.curr_commit: avg_rec_curr,
            },
            "routes": {
                "owner_count": len(owner_scenarios),
                "fallback_count": len(fallback_scenarios),
                "owner_curr_warm_p50_us": round(owner_c_p50, 1),
                "owner_ra_warm_p50_us": round(owner_ra_p50, 1),
                "owner_speedup_fair": owner_speedup,
                "fallback_curr_warm_p50_us": round(fallback_c_p50, 1),
                "fallback_ra_warm_p50_us": round(fallback_ra_p50, 1),
                "fallback_speedup_fair": fallback_speedup,
                "overall_curr_warm_p50_us": round(all_c_p50, 1),
                "overall_ra_warm_p50_us": round(all_ra_p50, 1),
                "overall_speedup": overall_speedup,
            }
        }
        all_comparisons[slug] = summary

        print(f"\n--- Summary for {slug} ---")
        print(f"  Empty responses:       {args.base_commit}: {empty_base} -> {args.curr_commit}: {empty_curr} (closed: {closed_empty_count}, regressions to empty: {regressions_to_empty})")
        print(f"  Total items returned:  {args.base_commit}: {total_items_base} -> {args.curr_commit}: {total_items_curr} (RA: {total_items_ra})")
        print(f"  Avg recall vs RA:      {args.base_commit}: {avg_rec_base}% -> {args.curr_commit}: {avg_rec_curr}%")
        print(f"  Owner share:           {len(owner_scenarios)}% ({owner_c_p50:.0f}µs RIDX vs {owner_ra_p50:.0f}µs RA -> {owner_speedup}x)")
        print(f"  Fallback share:        {len(fallback_scenarios)}% ({fallback_c_p50:.0f}µs RIDX vs {fallback_ra_p50:.0f}µs RA -> {fallback_speedup}x)")
        print(f"  Overall:               {all_c_p50:.0f}µs RIDX vs {all_ra_p50:.0f}µs RA -> {overall_speedup}x speedup")

        published_records.append({
            "repo": slug,
            "commit_ridx": args.curr_commit,
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
                "avg_recall_vs_ra_pct": avg_rec_curr,
                "base_avg_recall_vs_ra_pct": avg_rec_base,
                "empty_responses_ridx": empty_curr,
                "empty_responses_base": empty_base,
                "empty_responses_ra": empty_ra,
            },
            "scenarios": sc_published,
        })

    out_json = bench_dir / "published" / f"completion_regression_{args.base_commit}_vs_{args.curr_commit}.json"
    with open(out_json, "w", encoding="utf-8") as f:
        json.dump(all_comparisons, f, indent=2)

    out_jsonl = bench_dir / "published" / f"rerun_completion_{args.curr_commit}.jsonl"
    with open(out_jsonl, "w", encoding="utf-8") as f:
        for rec in published_records:
            f.write(json.dumps(rec) + "\n")

    print(f"\nPublished outputs generated successfully:")
    print(f"  Comparison report: {out_json}")
    print(f"  Published rerun:   {out_jsonl}")


if __name__ == "__main__":
    main()
