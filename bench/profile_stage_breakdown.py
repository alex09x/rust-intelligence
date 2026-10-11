# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Empirical stage breakdown profiler for completion requests.
Measures deref, find_defs, categorize, collect, and trait cache hit/miss across warm repetitions.
"""

import argparse
import json
import os
import re
import statistics
import subprocess
import sys
import time
from pathlib import Path

bench_dir = Path(__file__).resolve().parent
if str(bench_dir) not in sys.path:
    sys.path.insert(0, str(bench_dir))

from lsp_process import SimpleLspProcess
from provenance import find_default_repo


def set_active_variant(repo_dir: Path, variant_id: str):
    p = repo_dir / "target" / "ridx" / "ridx_manifest.json"
    with open(p, "r", encoding="utf-8") as f:
        d = json.load(f)
    d["active_variant_id"] = variant_id
    d["variant_dir"] = f"variants/{variant_id}"
    with open(p, "w", encoding="utf-8") as f:
        json.dump(d, f, indent=2)


def profile_scenario(
    repo_dir: Path,
    ridx_bin: Path,
    file_rel: str,
    line: int,
    char: int,
    reps: int = 20,
    taskset: str = "24-31",
):
    full_p = str((repo_dir / file_rel).resolve())
    params = {
        "textDocument": {"uri": f"file://{full_p}"},
        "position": {"line": line, "character": char},
    }

    env_backup = dict(os.environ)
    os.environ["RIDX_PROFILE_COMPLETION"] = "1"

    cmd = [str(ridx_bin), "lsp"]
    if taskset and sys.platform.startswith("linux"):
        cmd = ["taskset", "-c", taskset] + cmd

    srv = SimpleLspProcess(cmd, str(repo_dir))

    with open(full_p, "r", encoding="utf-8") as fp:
        content = fp.read()
    srv.notify(
        "textDocument/didOpen",
        {
            "textDocument": {
                "uri": f"file://{full_p}",
                "languageId": "rust",
                "version": 1,
                "text": content,
            }
        },
    )

    # 1 cold request
    cold_res, cold_dur, _ = srv.request("textDocument/completion", params, timeout=10.0)

    # N warm requests
    warm_durs = []
    res = None
    for _ in range(reps):
        res, dur, _ = srv.request("textDocument/completion", params, timeout=10.0)
        warm_durs.append(dur)

    items = res.get("items", []) if res else []
    item_count = len(items)

    srv.p.terminate()
    try:
        srv.p.wait(timeout=2.0)
    except Exception:
        srv.p.kill()

    os.environ.clear()
    os.environ.update(env_backup)

    with open(srv.stderr_path, "r", encoding="utf-8", errors="replace") as ef:
        stderr_lines = [line_str.strip() for line_str in ef]

    # Parse stderr lines
    prof_re = re.compile(
        r"\[RIDX_PROFILE\] deref_us=(\d+) find_defs_us=(\d+) \(defs=(\d+)\) cat_us=(\d+) collect_us=(\d+) total_us=(\d+)"
    )
    traits_re = re.compile(r"\[RIDX_PROFILE_TRAITS\] hit=(true|false) us=(\d+)")

    prof_entries = []
    for s_line in stderr_lines:
        m = prof_re.search(s_line)
        if m:
            prof_entries.append({
                "deref_us": int(m.group(1)),
                "find_defs_us": int(m.group(2)),
                "defs": int(m.group(3)),
                "cat_us": int(m.group(4)),
                "collect_us": int(m.group(5)),
                "total_us": int(m.group(6)),
            })

    traits_miss = []
    traits_hit = []
    for s_line in stderr_lines:
        m = traits_re.search(s_line)
        if m:
            is_hit = m.group(1) == "true"
            val = int(m.group(2))
            if is_hit:
                traits_hit.append(val)
            else:
                traits_miss.append(val)

    # First entry is cold request, next reps entries are warm requests
    cold_entry = prof_entries[0] if prof_entries else {}
    warm_entries = prof_entries[1:] if len(prof_entries) > 1 else prof_entries

    return {
        "item_count": item_count,
        "cold_dur_us": cold_dur,
        "warm_durs_us": warm_durs,
        "median_warm_dur_us": statistics.median(warm_durs) if warm_durs else 0,
        "cold_stage": cold_entry,
        "warm_stages": warm_entries,
        "median_deref_us": statistics.median([e["deref_us"] for e in warm_entries]) if warm_entries else 0,
        "median_find_defs_us": statistics.median([e["find_defs_us"] for e in warm_entries]) if warm_entries else 0,
        "median_cat_us": statistics.median([e["cat_us"] for e in warm_entries]) if warm_entries else 0,
        "median_collect_us": statistics.median([e["collect_us"] for e in warm_entries]) if warm_entries else 0,
        "median_total_owner_us": statistics.median([e["total_us"] for e in warm_entries]) if warm_entries else 0,
        "traits_miss_us": traits_miss[0] if traits_miss else 0,
        "median_traits_hit_us": statistics.median(traits_hit) if traits_hit else 0,
    }


def parse_args():
    parser = argparse.ArgumentParser(description="Profile RIDX completion stage latency breakdown.")
    parser.add_argument(
        "--repo-dir",
        type=Path,
        default=None,
        help="Path to repository root (defaults to auto-discovered bevy checkout)",
    )
    parser.add_argument(
        "--ridx-bin",
        type=Path,
        default=None,
        help="Path to ridx binary (defaults to target/release/ridx)",
    )
    parser.add_argument(
        "--canon-variant",
        default="dev-3f72a8c14f5b1a32",
        help="Canonical variant ID (default: dev-3f72a8c14f5b1a32)",
    )
    parser.add_argument(
        "--all-targets-variant",
        default="dev-61b2604f54cd0e0b",
        help="All-targets variant ID (default: dev-61b2604f54cd0e0b)",
    )
    parser.add_argument(
        "--file",
        default="crates/bevy_ecs/src/world/entity_access/mod.rs",
        help="File path relative to repository root (default: crates/bevy_ecs/src/world/entity_access/mod.rs)",
    )
    parser.add_argument(
        "--line",
        type=int,
        default=1579,
        help="0-based LSP line number (default: 1579)",
    )
    parser.add_argument(
        "--character",
        type=int,
        default=31,
        help="0-based LSP character position (default: 31)",
    )
    parser.add_argument(
        "--scenario-name",
        default="comp_dot_41_entity_mut",
        help="Scenario display name (default: comp_dot_41_entity_mut)",
    )
    parser.add_argument(
        "--reps",
        type=int,
        default=20,
        help="Number of warm repetitions (default: 20)",
    )
    parser.add_argument(
        "--taskset",
        default="24-31",
        help="CPU cores to bind with taskset on Linux (default: 24-31, empty to disable)",
    )
    return parser.parse_args()


def main():
    args = parse_args()
    repo_dir = (args.repo_dir or find_default_repo("bevy", bench_dir=bench_dir)).resolve()
    ridx_bin = (args.ridx_bin or (bench_dir.parent / "target" / "release" / "ridx")).resolve()

    if not repo_dir.is_dir():
        print(f"Error: repo_dir not found at {repo_dir}", file=sys.stderr)
        sys.exit(1)
    if not ridx_bin.is_file():
        print(f"Error: ridx_bin not found at {ridx_bin}", file=sys.stderr)
        sys.exit(1)

    canon_id = args.canon_variant
    all_targets_id = args.all_targets_variant

    print(f"Profiling Canonical variant ({canon_id})...")
    set_active_variant(repo_dir, canon_id)
    canon_res = profile_scenario(
        repo_dir,
        ridx_bin,
        args.file,
        args.line,
        args.character,
        reps=args.reps,
        taskset=args.taskset,
    )

    print(f"Profiling All-Targets variant ({all_targets_id})...")
    set_active_variant(repo_dir, all_targets_id)
    all_res = profile_scenario(
        repo_dir,
        ridx_bin,
        args.file,
        args.line,
        args.character,
        reps=args.reps,
        taskset=args.taskset,
    )

    # Always restore canonical
    set_active_variant(repo_dir, canon_id)

    print("\n" + "=" * 60)
    print(f"STAGE BREAKDOWN RESULTS (Median of {args.reps} warm reps, scenario: {args.scenario_name})")
    print("=" * 60)
    print(f"Items returned: Canon={canon_res['item_count']}, All-Targets={all_res['item_count']}")
    print(f"\nStage | Canonical ({canon_id}) | All-Targets ({all_targets_id})")
    print("-" * 65)
    print(f"1. resolve_deref_chain              | {canon_res['median_deref_us']:>6} µs | {all_res['median_deref_us']:>6} µs")
    print(f"2. find_definitions_by_owner_for_file | {canon_res['median_find_defs_us']:>6} µs | {all_res['median_find_defs_us']:>6} µs")
    print(f"3. categorize_definition            | {canon_res['median_cat_us']:>6} µs | {all_res['median_cat_us']:>6} µs")
    print(f"4. collect_prioritized_items        | {canon_res['median_collect_us']:>6} µs | {all_res['median_collect_us']:>6} µs")
    print(f"   Total try_owner_completion       | {canon_res['median_total_owner_us']:>6} µs | {all_res['median_total_owner_us']:>6} µs")
    print(f"   Overall Warm LSP Request (p50)   | {canon_res['median_warm_dur_us']:>6} µs | {all_res['median_warm_dur_us']:>6} µs")
    print("-" * 65)
    print("File Referenced Traits Cache:")
    print(f" - Cache Miss (Cold / First scan)   | {canon_res['traits_miss_us']:>6} µs | {all_res['traits_miss_us']:>6} µs")
    print(f" - Cache Hit (Warm / Median of {args.reps})  | {canon_res['median_traits_hit_us']:>6} µs | {all_res['median_traits_hit_us']:>6} µs")
    print("=" * 60)


if __name__ == "__main__":
    main()
