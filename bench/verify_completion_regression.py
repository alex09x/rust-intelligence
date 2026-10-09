# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Empirical regression and parity verification: baseline vs current commit vs rust-analyzer."""

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
from provenance import (
    find_default_repo,
    find_sysroot_cas_key,
    get_driver_hash,
    get_sysroot_std_metadata,
)

ROOT_CAUSES = {
    "comp_colon_41_RangeVisitor": "Shadowing: sysroot core::ops::range resolved before local mod range at serde_core/src/de/impls.rs:2463 (5 sysroot items vs 2 local).",
    "comp_colon_37_SocketAddr": "Re-export omission: std::net re-exports core::net::SocketAddr, but std.ridx excluded core re-exports (1 item vs 13).",
}


def load_baseline(baseline_file):
    records = {}
    with open(baseline_file, "r", encoding="utf-8") as f:
        for line in f:
            if line.strip():
                d = json.loads(line)
                records[d["repo"]] = d
    return records


def preopen_buffers(srv, repo_dir, scenarios):
    opened = set()
    for sc in scenarios:
        full_path = Path(repo_dir) / sc["location"][0]
        if full_path not in opened and full_path.is_file():
            opened.add(full_path)
            try:
                with open(full_path, "r", encoding="utf-8", errors="replace") as fp:
                    srv.notify("textDocument/didOpen", {
                        "textDocument": {"uri": f"file://{full_path}", "languageId": "rust", "version": 1, "text": fp.read()}
                    })
            except (OSError, UnicodeDecodeError):
                pass


def run_scenarios(srv, repo_dir, scenarios, is_ra=False, timeout=5.0):
    if is_ra and hasattr(srv, "wait_quiescent"):
        srv.wait_quiescent(timeout=10.0, settle_idle=0.5)

    results = []
    for idx, sc in enumerate(scenarios):
        full_path = Path(repo_dir) / sc["location"][0]
        params = {"textDocument": {"uri": f"file://{full_path}"}, "position": {"line": sc["location"][1] - 1, "character": sc["location"][2] - 1}}
        durs, first_res = [], None
        for it in range(3):
            res, dur, _ = srv.request("textDocument/completion", params, timeout=timeout)
            durs.append(dur)
            if it == 0:
                first_res = res
        items = first_res.get("items", []) if first_res else []
        results.append({
            "name": sc["name"], "location": sc["location"], "cold_us": durs[0],
            "warm_us": sorted(durs[1:])[len(durs[1:]) // 2] if len(durs) > 1 else durs[0],
            "count": len(items), "labels": [it.get("label", "").split("(")[0].strip() for it in items],
            "first_detail": items[0].get("detail", "") if items else "",
        })
        if (idx + 1) % 25 == 0:
            print(f"    Completed {idx + 1}/{len(scenarios)} scenarios...")
    return results





def calc_route_acc(scs, key_rec, key_prec, key_cnt):
    recs = [s[key_rec] for s in scs if s["ra_count"] > 0]
    precs = [s[key_prec] for s in scs if s[key_cnt] > 0 and s["ra_count"] > 0]
    return (round(sum(recs) / len(recs), 1) if recs else 0.0, round(sum(precs) / len(precs), 1) if precs else 0.0)


def parse_args():
    p = argparse.ArgumentParser(description="Empirical regression and parity verification")
    p.add_argument("--base-commit", default="b05240a", help="Baseline commit SHA")
    p.add_argument("--curr-commit", default="c308792", help="Current commit SHA under test")
    p.add_argument("--commit-ridx", help="Alias for --curr-commit")
    p.add_argument("--base-jsonl", help="Explicit path to baseline rerun jsonl")
    p.add_argument("--curr-jsonl", help="Explicit path to current rerun jsonl (replay mode)")
    p.add_argument("--replay", action="store_true", help="Replay directly from existing published jsonl")
    p.add_argument("--v2", action="store_true", help="Format and name report with v2 methodology")
    p.add_argument("--v3", action="store_true", help="Format and name report with v3 methodology")
    p.add_argument("--ridx-bin", help="Path to current ridx binary")
    p.add_argument("--ra-bin", help="Path to rust-analyzer binary")
    p.add_argument("--serde-dir", help="Path to serde repo")
    p.add_argument("--bevy-dir", help="Path to bevy repo")
    p.add_argument("--output-json", help="Path for comparison report json")
    p.add_argument("--output-jsonl", help="Path for output rerun jsonl")
    p.add_argument("--force", action="store_true", help="Allow overwriting existing published benchmark files")
    p.add_argument("--toolchain", default="1.98.1", help="Expected compiler and rust-analyzer toolchain version")
    p.add_argument("--allow-invalid-config", action="store_true", help="Allow running even if toolchain or sysroot invalid")
    return p.parse_args()


def calc_pct(data, p):
    return round(percentile(data, p), 1) if data else 0.0


def main():
    args = parse_args()
    curr_commit = args.commit_ridx if args.commit_ridx else args.curr_commit
    base_commit = args.base_commit
    v_suffix = "_v3" if args.v3 else ("_v2" if args.v2 else "")
    tc_target = args.toolchain
    os.environ["RUSTUP_TOOLCHAIN"] = tc_target

    out_json = Path(args.output_json) if args.output_json else (
        bench_dir / "published" / f"completion_regression_{base_commit}_vs_{curr_commit}{v_suffix}.json"
    )
    out_jsonl = Path(args.output_jsonl) if args.output_jsonl else (
        bench_dir / "published" / f"rerun_completion_{curr_commit}.jsonl"
    )
    base_file = Path(args.base_jsonl) if args.base_jsonl else (bench_dir / "published" / f"rerun_completion_{base_commit}.jsonl")
    curr_file = Path(args.curr_jsonl) if args.curr_jsonl else (
        bench_dir / "published" / f"rerun_completion_{curr_commit}.jsonl" if args.replay else None
    )

    base_data = load_baseline(base_file)
    curr_data = load_baseline(curr_file) if (curr_file and curr_file.is_file()) else None

    if not args.force:
        if out_json.exists():
            raise FileExistsError(f"Refusing to overwrite existing file without --force: {out_json}")
        if not curr_data and out_jsonl.exists():
            raise FileExistsError(f"Refusing to overwrite existing file without --force: {out_jsonl}")

    serde_dir = Path(args.serde_dir) if args.serde_dir else find_default_repo("serde")
    bevy_dir = Path(args.bevy_dir) if args.bevy_dir else find_default_repo("bevy")
    repos = [("serde-rs/serde", str(serde_dir)), ("bevyengine/bevy", str(bevy_dir))]

    ra_ver, tc_ver = f"rust-analyzer {tc_target}", f"rustc {tc_target}"
    if not curr_data:
        curr_bin = (
            Path(args.ridx_bin) if args.ridx_bin else (bench_dir.parent / "target" / "release" / "ridx")
        ).resolve()
        ra_custom = Path.home() / ".rustup" / "toolchains" / f"{tc_target}-x86_64-unknown-linux-gnu" / "bin" / "rust-analyzer"
        ra_bin = args.ra_bin if args.ra_bin else (str(ra_custom) if ra_custom.is_file() else "rust-analyzer")
        rustc_custom = Path.home() / ".rustup" / "toolchains" / f"{tc_target}-x86_64-unknown-linux-gnu" / "bin" / "rustc"
        rustc_bin = str(rustc_custom) if rustc_custom.is_file() else "rustc"
        ra_ver = subprocess.run([ra_bin, "--version"], capture_output=True, text=True, check=False).stdout.strip()
        tc_ver = subprocess.run([rustc_bin, "--version"], capture_output=True, text=True, check=False, env={**os.environ, "RUSTUP_TOOLCHAIN": tc_target}).stdout.strip()

    all_comparisons, published_records = {}, []

    for slug, rpath in repos:
        scenarios_base = base_data[slug]["scenarios"]
        print(f"\n[{slug}] Evaluating {len(scenarios_base)} scenarios for {curr_commit} vs {base_commit}...")

        if curr_data:
            c_rec = curr_data[slug]
            ra_ver, tc_ver, cas_key = c_rec.get("baseline_ra", ra_ver), c_rec.get("toolchain", tc_ver), c_rec.get("sysroot_cas_key")
            driver_hash = c_rec.get("driver_hash")
            std_mtime = c_rec.get("std_ridx_mtime")
            std_sha = c_rec.get("std_ridx_hash")
            res_scs = c_rec["scenarios"]
        else:
            srv_curr = SimpleLspProcess([str(curr_bin), "lsp"], rpath)
            preopen_buffers(srv_curr, rpath, scenarios_base)
            res_curr = run_scenarios(srv_curr, rpath, scenarios_base, is_ra=False, timeout=5.0)
            srv_curr.notify("exit", {})

            srv_ra = SimpleLspProcess([ra_bin], rpath, is_ra=True)
            preopen_buffers(srv_ra, rpath, scenarios_base)
            res_ra = run_scenarios(srv_ra, rpath, scenarios_base, is_ra=True, timeout=10.0)
            srv_ra.notify("exit", {})

            cas_key = find_sysroot_cas_key(rpath)
            driver_hash = get_driver_hash(curr_bin)
            std_mtime, std_sha = get_sysroot_std_metadata(cas_key)
            res_scs = None

        is_invalid_config = not cas_key or (tc_target not in tc_ver)
        if is_invalid_config and not args.allow_invalid_config:
            raise RuntimeError(
                f"Invalid benchmark configuration for {slug}: toolchain={tc_ver!r} (expected {tc_target}), sysroot_cas_key={cas_key!r}. "
                "Use --allow-invalid-config to override."
            )

        empty_b = empty_c = empty_ra = reg_empty = closed_cnt = ra_zero = 0
        closed_names = []
        tot_b = tot_c = tot_ra = 0
        recs_b, recs_c, precs_b, precs_c = [], [], [], []
        recall_regs = []
        owner_scs, fallback_scs, sc_pub = [], [], []
        migrated_to_owner = 0

        for idx, sc_b in enumerate(scenarios_base):
            base_cnt, base_ra_cnt = sc_b["ridx_count"], sc_b["ra_count"]
            if curr_data:
                c_sc = res_scs[idx]
                curr_cnt, ra_cnt = c_sc["ridx_count"], c_sc["ra_count"]
                rec_c, prec_c = c_sc["recall_pct"], c_sc.get("precision_pct", 0.0)
                warm_c, cold_c, warm_r, cold_r = c_sc["ridx_warm_us"], c_sc["ridx_cold_us"], c_sc["ra_warm_us"], c_sc["ra_cold_us"]
                route = c_sc["route"]
            else:
                c, r = res_curr[idx], res_ra[idx]
                curr_cnt, ra_cnt = c["count"], r["count"]
                labels_c, labels_r = set(c["labels"]), set(r["labels"])
                matched_c = len(labels_c & labels_r)
                rec_c = (matched_c / len(labels_r) * 100.0) if labels_r else 0.0
                prec_c = (matched_c / len(labels_c) * 100.0) if labels_c else 0.0
                warm_c, cold_c, warm_r, cold_r = round(c["warm_us"]), round(c["cold_us"]), round(r["warm_us"]), round(r["cold_us"])
                first_det = c["first_detail"]
                is_owner = "::" in first_det.rsplit(" (in ", 1)[1].rstrip(")") if " (in " in first_det else False
                route = "owner_path" if is_owner else "fallback"

            if base_cnt == 0: empty_b += 1
            if curr_cnt == 0: empty_c += 1
            if ra_cnt == 0: empty_ra += 1; ra_zero += 1

            if base_cnt > 0 and curr_cnt == 0: reg_empty += 1
            if base_cnt == 0 and curr_cnt > 0: closed_cnt += 1; closed_names.append(sc_b["name"])

            tot_b += base_cnt; tot_c += curr_cnt; tot_ra += ra_cnt

            if ra_cnt > 0:
                recs_b.append(sc_b["recall_pct"])
                recs_c.append(rec_c)
                diff_rec = sc_b["recall_pct"] - rec_c
                if diff_rec > 5.0:
                    reg_entry = {
                        "name": sc_b["name"], "base_recall_pct": sc_b["recall_pct"],
                        "curr_recall_pct": round(rec_c, 1), "diff_pp": round(diff_rec, 1),
                        "base_count": base_cnt, "curr_count": curr_cnt, "ra_count": ra_cnt,
                    }
                    if sc_b["name"] in ROOT_CAUSES:
                        reg_entry["root_cause"] = ROOT_CAUSES[sc_b["name"]]
                    recall_regs.append(reg_entry)

            if base_cnt > 0:
                prec_b = sc_b["precision_pct"] if "precision_pct" in sc_b else (round((sc_b["recall_pct"] / 100.0) * base_ra_cnt) / base_cnt * 100.0)
                if base_ra_cnt > 0:
                    precs_b.append(prec_b)
            else:
                prec_b = 0.0

            if curr_cnt > 0 and ra_cnt > 0:
                precs_c.append(prec_c)

            if sc_b.get("route") == "fallback" and route == "owner_path":
                migrated_to_owner += 1

            entry_pub = {
                "name": sc_b["name"], "target": sc_b["target"], "kind": sc_b["kind"], "location": sc_b["location"],
                "route": route, "ridx_warm_us": warm_c, "ridx_cold_us": cold_c, "ra_warm_us": warm_r, "ra_cold_us": cold_r,
                "recall_pct": round(rec_c, 1), "precision_pct": round(prec_c, 1),
                "ridx_count": curr_cnt, "ra_count": ra_cnt, "base_count": base_cnt,
                "base_recall_pct": sc_b["recall_pct"], "base_precision_pct": round(prec_b, 1),
            }
            sc_pub.append(entry_pub)
            (owner_scs if route == "owner_path" else fallback_scs).append(entry_pub)

        owner_c_warm = [s["ridx_warm_us"] for s in owner_scs]
        owner_ra_warm = [s["ra_warm_us"] for s in owner_scs]
        fallback_c_warm = [s["ridx_warm_us"] for s in fallback_scs]
        fallback_ra_warm = [s["ra_warm_us"] for s in fallback_scs]
        all_c_warm = [s["ridx_warm_us"] for s in sc_pub]
        all_ra_warm = [s["ra_warm_us"] for s in sc_pub]

        o_c_p50, o_c_p95 = calc_pct(owner_c_warm, 50.0), calc_pct(owner_c_warm, 95.0)
        o_ra_p50, o_ra_p95 = calc_pct(owner_ra_warm, 50.0), calc_pct(owner_ra_warm, 95.0)
        f_c_p50, f_c_p95 = calc_pct(fallback_c_warm, 50.0), calc_pct(fallback_c_warm, 95.0)
        f_ra_p50, f_ra_p95 = calc_pct(fallback_ra_warm, 50.0), calc_pct(fallback_ra_warm, 95.0)
        all_c_p50, all_c_p95 = calc_pct(all_c_warm, 50.0), calc_pct(all_c_warm, 95.0)
        all_ra_p50, all_ra_p95 = calc_pct(all_ra_warm, 50.0), calc_pct(all_ra_warm, 95.0)

        o_speedup = round(o_ra_p50 / max(1.0, o_c_p50), 2) if o_c_p50 > 0 else 0.0
        f_speedup = round(f_ra_p50 / max(1.0, f_c_p50), 2) if f_c_p50 > 0 else 0.0
        tot_speedup = round(all_ra_p50 / max(1.0, all_c_p50), 2)

        avg_rec_b = round(sum(recs_b) / len(recs_b), 1) if recs_b else 0.0
        avg_rec_c = round(sum(recs_c) / len(recs_c), 1) if recs_c else 0.0
        avg_prec_b = round(sum(precs_b) / len(precs_b), 1) if precs_b else 0.0
        avg_prec_c = round(sum(precs_c) / len(precs_c), 1) if precs_c else 0.0

        o_rec_c, o_prec_c = calc_route_acc(owner_scs, "recall_pct", "precision_pct", "ridx_count")
        o_rec_b, o_prec_b = calc_route_acc(owner_scs, "base_recall_pct", "base_precision_pct", "base_count")
        f_rec_c, f_prec_c = calc_route_acc(fallback_scs, "recall_pct", "precision_pct", "ridx_count")
        f_rec_b, f_prec_b = calc_route_acc(fallback_scs, "base_recall_pct", "base_precision_pct", "base_count")

        summary = {
            "repo": slug, "toolchain": tc_ver, "sysroot_cas_key": cas_key,
            "driver_hash": driver_hash, "std_ridx_mtime": std_mtime, "std_ridx_hash": std_sha,
            "invalid_config": is_invalid_config,
            "total_scenarios": len(scenarios_base),
            "empty_responses": {base_commit: empty_b, curr_commit: empty_c, "rust_analyzer_1_98_1": empty_ra},
            "closed_empty_responses": {"count": closed_cnt, "scenarios": closed_names},
            "regressions_to_empty": reg_empty,
            "total_candidates": {base_commit: tot_b, curr_commit: tot_c, "rust_analyzer_1_98_1": tot_ra},
            "ra_zero_oracle_count": ra_zero,
            "avg_recall_vs_ra": {
                "description": "Calculated strictly over scenarios where ra_count > 0",
                base_commit: avg_rec_b, f"{base_commit}_n": len(recs_b),
                curr_commit: avg_rec_c, f"{curr_commit}_n": len(recs_c),
            },
            "avg_precision_vs_ra": {
                "description": "Calculated strictly over non-empty responses where ra_count > 0",
                base_commit: avg_prec_b, f"{base_commit}_n": len(precs_b),
                curr_commit: avg_prec_c, f"{curr_commit}_n": len(precs_c),
            },
            "recall_regressions_gt_5pp": recall_regs,
            "routes": {
                "owner_count": len(owner_scs), "fallback_count": len(fallback_scs),
                "migrated_from_fallback_to_owner": migrated_to_owner,
                "owner_accuracy": {"base_recall_pct": o_rec_b, "curr_recall_pct": o_rec_c, "base_precision_pct": o_prec_b, "curr_precision_pct": o_prec_c},
                "fallback_accuracy": {"base_recall_pct": f_rec_b, "curr_recall_pct": f_rec_c, "base_precision_pct": f_prec_b, "curr_precision_pct": f_prec_c},
                "owner_curr_warm_p50_us": o_c_p50, "owner_curr_warm_p95_us": o_c_p95,
                "owner_ra_warm_p50_us": o_ra_p50, "owner_ra_warm_p95_us": o_ra_p95,
                "owner_speedup_fair": o_speedup,
                "fallback_curr_warm_p50_us": f_c_p50, "fallback_curr_warm_p95_us": f_c_p95,
                "fallback_ra_warm_p50_us": f_ra_p50, "fallback_ra_warm_p95_us": f_ra_p95,
                "fallback_speedup_fair": f_speedup,
                "overall_curr_warm_p50_us": all_c_p50, "overall_curr_warm_p95_us": all_c_p95,
                "overall_ra_warm_p50_us": all_ra_p50, "overall_ra_warm_p95_us": all_ra_p95,
                "overall_speedup": tot_speedup,
            },
        }
        all_comparisons[slug] = summary

        print(f"\n--- Summary for {slug} ---")
        print(f"  Empty responses:       {base_commit}: {empty_b} -> {curr_commit}: {empty_c} (closed: {closed_cnt})")
        print(f"  Avg recall (ra>0):     {base_commit}: {avg_rec_b}% (n={len(recs_b)}) -> {curr_commit}: {avg_rec_c}% (n={len(recs_c)}) [ra_zero={ra_zero}]")
        print(f"  Avg precision (cnt>0, ra>0): {base_commit}: {avg_prec_b}% (n={len(precs_b)}) -> {curr_commit}: {avg_prec_c}% (n={len(precs_c)})")
        print(f"  Owner accuracy:        recall {o_rec_b}% -> {o_rec_c}% | precision {o_prec_b}% -> {o_prec_c}% (n={len(owner_scs)})")
        print(f"  Fallback accuracy:     recall {f_rec_b}% -> {f_rec_c}% | precision {f_prec_b}% -> {f_prec_c}% (n={len(fallback_scs)})")
        print(f"  Recall regressions:    {len(recall_regs)} scenarios > 5pp")
        print(f"  Owner:    p50 {o_c_p50:.0f}µs RIDX vs {o_ra_p50:.0f}µs RA ({o_speedup}x) | p95 {o_c_p95:.0f}µs vs {o_ra_p95:.0f}µs")
        print(f"  Fallback: p50 {f_c_p50:.0f}µs RIDX vs {f_ra_p50:.0f}µs RA ({f_speedup}x) | p95 {f_c_p95:.0f}µs vs {f_ra_p95:.0f}µs")
        print(f"  Overall:  p50 {all_c_p50:.0f}µs RIDX vs {all_ra_p50:.0f}µs RA ({tot_speedup}x) | p95 {all_c_p95:.0f}µs vs {all_ra_p95:.0f}µs")

        if not curr_data:
            published_records.append({
                "repo": slug, "commit_ridx": curr_commit, "baseline_ra": ra_ver,
                "toolchain": tc_ver, "sysroot_cas_key": cas_key,
                "driver_hash": driver_hash, "std_ridx_mtime": std_mtime, "std_ridx_hash": std_sha,
                "invalid_config": is_invalid_config,
                "node": "booster (AMD Ryzen 9 7950X, 16 cores / 32 threads, 128 GB RAM)",
                "taskset": "24-31", "seed": 42,
                "timestamp": datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z"),
                "sample_count": len(scenarios_base), "capability": "completion",
                "metrics": {
                    "ridx_completion_warm_p50_us": all_c_p50, "ridx_completion_warm_p95_us": all_c_p95,
                    "ridx_owner_warm_p50_us": o_c_p50, "ridx_owner_warm_p95_us": o_c_p95,
                    "ridx_fallback_warm_p50_us": f_c_p50, "ridx_fallback_warm_p95_us": f_c_p95,
                    "ra_matching_owner_warm_p50_us": o_ra_p50, "ra_matching_fallback_warm_p50_us": f_ra_p50,
                    "ra_overall_warm_p50_us": all_ra_p50,
                    "owner_p50_speedup_vs_ra": o_speedup, "fallback_p50_speedup_vs_ra": f_speedup, "overall_p50_speedup_vs_ra": tot_speedup,
                    "owner_share_pct": round(len(owner_scs) / len(scenarios_base) * 100.0, 1),
                    "fallback_share_pct": round(len(fallback_scs) / len(scenarios_base) * 100.0, 1),
                    "avg_recall_vs_ra_pct": avg_rec_c, "avg_precision_vs_ra_pct": avg_prec_c,
                    "base_avg_recall_vs_ra_pct": avg_rec_b, "base_avg_precision_vs_ra_pct": avg_prec_b,
                    "empty_responses_ridx": empty_c, "empty_responses_base": empty_b, "empty_responses_ra": empty_ra,
                },
                "scenarios": sc_pub,
            })

    with open(out_json, "w", encoding="utf-8") as f:
        json.dump(all_comparisons, f, indent=2)

    if published_records:
        with open(out_jsonl, "w", encoding="utf-8") as f:
            for rec in published_records:
                f.write(json.dumps(rec) + "\n")

    print(f"\nPublished report generated: {out_json}")


if __name__ == "__main__":
    main()
