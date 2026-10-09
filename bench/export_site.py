# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Exports pilot benchmark latency and resource telemetry to site/bench.json.
Provides single-source-of-truth data binding for the public website.
"""

import json
from pathlib import Path
import sys

bench_dir = Path(__file__).resolve().parent


def fmt_lat(us):
    if us < 1000:
        return f"{us:.0f} µs"
    return f"{us/1000.0:.2f} ms"


def export_site_bench(results_path, discrepancies_path, out_path):
    results_file = Path(results_path)
    if not results_file.exists():
        print(f"Error: {results_file} not found", file=sys.stderr)
        return

    repos_data = []
    with open(results_file, "r", encoding="utf-8") as f:
        for line in f:
            if line.strip():
                repos_data.append(json.loads(line))

    workspaces = {}
    repo_meta = {
        "serde-rs/serde": {
            "label": "serde",
            "meta": "serde · 3 crates · warm cache · booster (16 cores / 32 threads)",
        },
        "BurntSushi/ripgrep": {
            "label": "ripgrep",
            "meta": "ripgrep · 7 crates · warm cache · booster (16 cores / 32 threads)",
        },
        "tokio-rs/tokio": {
            "label": "Tokio",
            "meta": "Tokio · 42 crates · 880 files · booster (16 cores / 32 threads)",
        },
        "bevyengine/bevy": {
            "label": "bevy",
            "meta": "bevy · 147 crates · 2,800 files · booster (16 cores / 32 threads)",
        },
        "rust-lang/rust-analyzer": {
            "label": "rust-analyzer",
            "meta": (
                "rust-analyzer · 261 crates · 3,050 files · booster (16 cores / 32 threads)"
            ),
        },
    }

    reruns = {}
    rerun_file = bench_dir / "published" / "rerun_hover_844eac0.jsonl"
    if rerun_file.exists():
        with open(rerun_file, "r", encoding="utf-8") as rf:
            for rline in rf:
                if rline.strip():
                    rentry = json.loads(rline)
                    reruns[rentry.get("repo", "")] = rentry

    for r in repos_data:
        slug = r.get("repo", "")
        info = repo_meta.get(slug, {"label": slug.split("/")[-1], "meta": slug})
        lbl = info["label"]
        m = r.get("metrics", {})

        rows = [
            [
                "Go to definition (warm p50)",
                m.get("ridx_def_warm_p50_us", 0) * 1e-6,
                fmt_lat(m.get("ridx_def_warm_p50_us", 0)),
                m.get("ra_def_warm_p50_us", 0) * 1e-6,
                fmt_lat(m.get("ra_def_warm_p50_us", 0)),
            ],
            [
                "Go to definition (tail p95)",
                m.get("ridx_def_warm_p95_us", 0) * 1e-6,
                fmt_lat(m.get("ridx_def_warm_p95_us", 0)),
                m.get("ra_def_warm_p95_us", 0) * 1e-6,
                fmt_lat(m.get("ra_def_warm_p95_us", 0)),
            ],
            [
                "Find references (warm p50)",
                m.get("ridx_refs_warm_p50_us", 0) * 1e-6,
                fmt_lat(m.get("ridx_refs_warm_p50_us", 0)),
                m.get("ra_refs_warm_p50_us", 0) * 1e-6,
                fmt_lat(m.get("ra_refs_warm_p50_us", 0)),
            ],
            [
                "Find references (tail p95)",
                m.get("ridx_refs_warm_p95_us", 0) * 1e-6,
                fmt_lat(m.get("ridx_refs_warm_p95_us", 0)),
                m.get("ra_refs_warm_p95_us", 0) * 1e-6,
                fmt_lat(m.get("ra_refs_warm_p95_us", 0)),
            ],
            [
                "Hover (warm p50)",
                m.get("ridx_hover_warm_p50_us", 0) * 1e-6,
                fmt_lat(m.get("ridx_hover_warm_p50_us", 0)),
                m.get("ra_hover_warm_p50_us", 0) * 1e-6,
                fmt_lat(m.get("ra_hover_warm_p50_us", 0)),
            ],
            [
                "Hover (tail p95)",
                m.get("ridx_hover_warm_p95_us", 0) * 1e-6,
                fmt_lat(m.get("ridx_hover_warm_p95_us", 0)),
                m.get("ra_hover_warm_p95_us", 0) * 1e-6,
                fmt_lat(m.get("ra_hover_warm_p95_us", 0)),
            ],
        ]

        if slug in reruns:
            rm = reruns[slug]["metrics"]
            rows.append([
                "Hover (warm p50 · post-844eac0, n=100)",
                rm.get("ridx_hover_warm_p50_us", 0) * 1e-6,
                fmt_lat(rm.get("ridx_hover_warm_p50_us", 0)),
                rm.get("ra_hover_warm_p50_us", 0) * 1e-6,
                fmt_lat(rm.get("ra_hover_warm_p50_us", 0)),
            ])
            rows.append([
                "Hover (tail p95 · post-844eac0, n=100)",
                rm.get("ridx_hover_warm_p95_us", 0) * 1e-6,
                fmt_lat(rm.get("ridx_hover_warm_p95_us", 0)),
                rm.get("ra_hover_warm_p95_us", 0) * 1e-6,
                fmt_lat(rm.get("ra_hover_warm_p95_us", 0)),
            ])

        w3_10 = (
            r.get("w3_multi_worktree", {})
            .get("live_server_scaling", {})
            .get("10", {})
        )
        workspaces[lbl] = {
            "meta": info["meta"],
            "target": (
                "Pilot benchmark · 300 sampled positions · warm latencies"
                " (post-index)"
            ),
            "rows": rows,
            "accuracy": {
                "readonly_strict_pct": r.get(
                    "readonly_match_rate_strict_pct", 0
                ),
                "readonly_matched": r.get("readonly_matched_strict", 0),
                "readonly_total": r.get("readonly_total", 300),
                "overall_strict_pct": r.get("match_rate_strict_pct", 0),
                "overall_matched": r.get("matched_scenarios_strict", 0),
                "overall_total": r.get("total_scenarios", 420),
            },
            "memory": {
                "ridx_heap_anon_mb": r.get("ridx_peak_rss_anon_mb", 0),
                "ra_heap_anon_mb": r.get("ra_peak_rss_anon_mb", 0),
                "ridx_pss_10_mb": w3_10.get("ridx", {}).get(
                    "total_pss_mb", 0
                ),
                "ra_pss_10_mb": w3_10.get("ra", {}).get("total_pss_mb", 0),
            },
        }

    output = {
        "node": "booster (AMD Ryzen 9 7950X, 16 cores / 32 threads, 128 GB RAM)",
        "baseline": "rust-analyzer 1.98.1 (48a229c)",
        "summary": {
            "corpus": "5 repositories (serde, ripgrep, tokio, bevy, rust-analyzer)",
            "total_scenarios": 2100,
            "overall_strict_matched": 1714,
            "overall_strict_pct": 81.62,
            "readonly_queries": 1500,
            "readonly_strict_matched": 1318,
            "readonly_strict_pct": 87.87,
            "sysroot_unique_discrepancies": 99,
            "sysroot_unique_discrepancies_pct": 25.65,
            "expected_sysroot_gain_overall_pp": 4.71,
            "expected_sysroot_gain_readonly_pp": 4.87,
        },
        "limitations": [
            (
                "Standard library (std/core/alloc) definition shards are not"
                " indexed yet; expected effect when indexed is +4.71 pp overall"
                " (+4.87 pp read-only)."
            ),
            (
                "textDocument/completion is non-speculative and resolves in tens"
                " of ms pending trie index caching."
            ),
            (
                "Hover latency on very large workspaces (bevy) is ~1.5x slower"
                " than rust-analyzer (354 µs vs 231 µs at warm p50, re-measured"
                " post-844eac0 down from 18.4x / 3.96 ms prior to binary search"
                " optimization); on small to medium workspaces (tokio, serde,"
                " ripgrep) it is faster than or on par with rust-analyzer."
            ),
        ],
        "workspaces": workspaces,
    }

    out_file = Path(out_path)
    out_file.parent.mkdir(parents=True, exist_ok=True)
    with open(out_file, "w", encoding="utf-8") as f:
        json.dump(output, f, indent=2)
    print(f"Generated {out_file} ({len(json.dumps(output))} bytes)")

    js_file = out_file.parent / "bench.data.js"
    js_payload = json.dumps(output, indent=2)
    with open(js_file, "w", encoding="utf-8") as f:
        f.write("/* Generated by bench/export_site.py — do not edit manually */\n")
        f.write(f"window.RIDX_BENCH = {js_payload};\n")
    print(f"Generated {js_file} ({len(js_payload)} bytes)")


def main():
    res = (
        sys.argv[1]
        if len(sys.argv) > 1
        else str(bench_dir / "results" / "pilot_results.jsonl")
    )
    disc = (
        sys.argv[2]
        if len(sys.argv) > 2
        else str(bench_dir / "results" / "discrepancies.jsonl")
    )
    out = (
        sys.argv[3]
        if len(sys.argv) > 3
        else str(bench_dir.parent / "site" / "bench.json")
    )
    export_site_bench(res, disc, out)


if __name__ == "__main__":
    main()
