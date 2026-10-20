# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Exports the published matrix runs to site/bench.json and site/bench.data.js: headline latencies per
repository, peak memory, coverage counters and the two notes the page shows. Every value comes from
bench/published/matrix_<repo>_booster.json; the page types no number by hand.
"""

import argparse
import json
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

from matrix_headline import (CANONICAL, HEADLINE_METHODS, REQUEST_KEYS, SITE_METHODS, answered,
                             by_method, defect_notes, distinct, fmt_us, load_published,
                             ra_not_implemented, ridx_silent, seconds, short, where)

Runs = List[Tuple[str, Dict[str, Any]]]
LABELS = {"serde": "serde", "ripgrep": "ripgrep", "tokio": "Tokio", "bevy": "bevy",
          "rust-analyzer": "rust-analyzer"}
PERCENTILES = [("warm_p50_us", "warm p50"), ("warm_p99_us", "tail p99")]


def _node_name(data: Dict[str, Any]) -> str:
    return str(data.get("node", "n/a")).split(" (")[0]


def workspace_entry(key: str, data: Dict[str, Any]) -> Dict[str, Any]:
    ridx, ra = data["ridx"], data["rust_analyzer"]
    titles = dict(HEADLINE_METHODS)
    rows = []
    for mkey in SITE_METHODS:
        for field, suffix in PERCENTILES:
            r = ridx["methods"].get(mkey, {}).get(field)
            a = ra["methods"].get(mkey, {}).get(field)
            rows.append([f"{titles[mkey]} ({suffix})", seconds(r), fmt_us(r), seconds(a), fmt_us(a)])
    return {
        "repo": data.get("repo", CANONICAL.get(key, key)),
        "meta": f"{LABELS.get(key, key)} · {data.get('ridx_index_shards', 'n/a')} index shards · "
                f"RIDX {short(data.get('ridx_commit'))} · {_node_name(data)}",
        "target": f"LSP matrix · {data.get('sample_count_per_method', 'n/a')} items per method · "
                  f"warm latencies · corpus {short(data.get('repo_commit'))}",
        "rows": rows,
        "memory": {"ridx_peak_rss_mb": ridx.get("peak_rss_mb"), "ra_peak_rss_mb": ra.get("peak_rss_mb")},
        "coverage": {"ridx_answered": answered(ridx["methods"]), "ra_answered": answered(ra["methods"]),
                     "request_methods": len(REQUEST_KEYS)},
    }


def method_note(runs: Runs) -> str:
    first = runs[0][1]
    return (f"{len(runs)} repositories, {first.get('total_methods', 'n/a')} LSP methods, "
            f"{first.get('sample_count_per_method', 'n/a')} sampled items per method "
            f"({int(first.get('total_tests_per_server', 0)):,} requests per server and repository, "
            f"seed {first.get('seed', 'n/a')}); RIDX {short(first.get('ridx_commit'))}, driver "
            f"{first.get('driver_hash', 'n/a')}, {first.get('ra_version', 'n/a')}; node {_node_name(first)}, "
            f"CPU set {first.get('taskset', 'n/a')}. Each item is asked three times; the first answer is "
            "the cold measurement and the mean of the other two the warm one. Bars show the warm p50 "
            "and p99 over the items; the scale is logarithmic.")


def limitations(runs: Runs) -> List[str]:
    total = len(runs)
    notes = ["Outside mutation case B nothing in the matrix checks that an answer is correct: a / b / c "
             "count non-empty, empty and failed answers (methodology, sections 5 and 7)."]
    notes += defect_notes(code=False)
    for method, repos in by_method(runs, ridx_silent).items():
        notes.append(f"RIDX returned no non-empty answer for {method} on {where(repos, total)} "
                     "while rust-analyzer did.")
    not_impl = by_method(runs, ra_not_implemented)
    if not_impl:
        notes.append("rust-analyzer answers -32601 (method not found) for: "
                     + ", ".join(f"{m} ({where(r, total)})" for m, r in not_impl.items()) + ".")
    rss_r = [d["ridx"].get("peak_rss_mb", 0.0) for _, d in runs]
    rss_a = [d["rust_analyzer"].get("peak_rss_mb", 0.0) for _, d in runs]
    notes.append(f"Peak RSS over the run: RIDX {min(rss_r):.0f} to {max(rss_r):.0f} MB, "
                 f"rust-analyzer {min(rss_a):.0f} to {max(rss_a):.0f} MB across the repositories "
                 "(RIDX maps its index files, so its RSS includes page-cache pages).")
    notes.append("Requests cancelled on the client timeout: RIDX "
                 f"{sum(int(d.get('ridx_cancelled_on_timeout', 0)) for _, d in runs)}, rust-analyzer "
                 f"{sum(int(d.get('ra_cancelled_on_timeout', 0)) for _, d in runs)}.")
    return notes


def export_site_bench(pub_dir: Path, out_path: Path) -> Dict[str, Any]:
    runs = load_published(pub_dir)
    output: Dict[str, Any] = {
        "generated_by": "bench/export_site.py",
        "source": "bench/published/matrix_<repo>_booster.json",
        "node": ", ".join(distinct(runs, "node")) if runs else "n/a",
        "baseline": ", ".join(distinct(runs, "ra_version")) if runs else "n/a",
        "ridx_commit": ", ".join(distinct(runs, "ridx_commit")) if runs else "n/a",
        "driver_hash": ", ".join(distinct(runs, "driver_hash")) if runs else "n/a",
        "method_note": method_note(runs) if runs else "",
        "limitations": limitations(runs) if runs else [],
        "workspaces": {LABELS.get(key, key): workspace_entry(key, data) for key, data in runs},
    }
    out_path.parent.mkdir(parents=True, exist_ok=True)
    payload = json.dumps(output, indent=2, ensure_ascii=False)
    out_path.write_text(payload + "\n", encoding="utf-8")
    js_file = out_path.parent / "bench.data.js"
    js_file.write_text("/* Generated by bench/export_site.py from bench/published/matrix_<repo>_booster.json. "
                       "Do not edit by hand. */\n" + f"window.RIDX_BENCH = {payload};\n", encoding="utf-8")
    print(f"Wrote {out_path} and {js_file} ({len(output['workspaces'])} repositories)")
    return output


def main(argv: Optional[List[str]] = None) -> None:
    parser = argparse.ArgumentParser(description="Export published matrix runs to site/bench.json")
    parser.add_argument("--pub", default=str(Path(__file__).resolve().parent / "published"))
    parser.add_argument("--out", default=str(Path(__file__).resolve().parents[1] / "site" / "bench.json"))
    args = parser.parse_args(argv)
    export_site_bench(Path(args.pub), Path(args.out))


if __name__ == "__main__":
    main()
