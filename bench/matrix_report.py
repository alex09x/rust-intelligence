# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Renders docs/BENCHMARK_MATRIX.md from the published 300 x 78 LSP matrix runs.

Every number in the report is read from bench/published/matrix_<repo>_booster.json;
nothing is computed from prose. Latencies are warm p50 / p99 in milliseconds.
"""

import argparse
import json
from pathlib import Path
from typing import Any, Dict, List, Optional

REPO_ORDER = [
    ("ripgrep", "BurntSushi/ripgrep"), ("tokio", "tokio-rs/tokio"),
    ("rust-analyzer", "rust-lang/rust-analyzer"), ("bevy", "bevyengine/bevy"), ("serde", "serde-rs/serde"),
]

PROVENANCE_ROWS = [
    ("RIDX commit", "ridx_commit"), ("RIDX binary dirty", "ridx_binary_dirty"),
    ("Driver hash", "driver_hash"), ("Driver build commit", "driver_build_commit"),
    ("Corpus commit", "repo_commit"), ("Sysroot CAS key", "sysroot_cas_key"),
    ("Index shards", "ridx_index_shards"), ("rustc", "rustc"), ("rust-analyzer", "ra_version"),
    ("Node", "node"), ("CPU set", "taskset"), ("Seed", "seed"),
    ("Samples per method", "sample_count_per_method"), ("Requests per server", "total_tests_per_server"),
    ("Started", "timestamp_start"), ("Finished", "timestamp_end"),
    ("RIDX suite duration (s)", "ridx_duration_sec"), ("rust-analyzer suite duration (s)", "ra_duration_sec"),
    ("rust-analyzer initial wait (ms)", "ra_initial_wait_ms"),
    ("rust-analyzer canary wait (ms) / attempts", None),
    ("rust-analyzer pre-query canary wait (ms) / attempts", None),
]

KNOWN_HARNESS_DEFECTS = [
    ("18088fc", "`$/cancelRequest` scenarios reused numeric ids that collided with query ids; "
     "string ids are used since this commit."),
    ("b704408", "synchronization scenarios left overlays open, so later queries ran against "
     "modified buffers; each scenario now opens and closes its own document."),
    ("9ebb233", "queries started as soon as rust-analyzer reported `quiescent`, before its "
     "`$/progress` work was finished; the runner now waits for all progress tokens to end."),
    ("e23e23d", "rust-analyzer starts a second indexing pass after the first `quiescent`; "
     "readiness is now confirmed by a `textDocument/definition` canary on the first three "
     "sampled positions (two consecutive non-empty answers)."),
    ("d3b7810", "the synchronization block (ids 8-14) makes rust-analyzer reload the workspace and "
     "answer `textDocument/definition` with empty results for a few seconds (measured on tokio: "
     "229/300 non-empty before the block, 0/300 right after, 229/300 sixty seconds later); the "
     "runner now re-establishes readiness before the first query method."),
]

NOTES = [
    "`-32801` is `ContentModified`: the server reports that re-analysis is in progress.",
    "RIDX `textDocument/documentSymbol` returns `[]` for files not opened via `didOpen`; the "
    "benchmark does not open files before symbol queries, so the RIDX documentSymbol latency is "
    "not comparable with rust-analyzer's.",
    "The readiness canary issues `textDocument/definition` on the first three sampled positions, so "
    "the rust-analyzer cold latency for those three positions out of 300 is measured on a warm cache.",
    "`a` counts non-empty answers, `b` empty answers (`null`, `[]`, empty hover), `c` failed "
    "scenarios; `errors` counts error responses, which can exceed `c` because a scenario sends "
    "several requests.",
]


def _ms(v: Any) -> str:
    if isinstance(v, (int, float)):
        return f"{v / 1000.0:.1f}"
    return "n/a"


def _codes(ec: Dict[str, Any]) -> str:
    return ", ".join(f"{k}: {v}" for k, v in ec.items()) if ec else "-"


def _abc(m: Dict[str, Any]) -> str:
    if "a_non_empty" not in m:
        return f"{m.get('tests_run', 0)} run"
    return f"{m.get('a_non_empty', 0)} / {m.get('b_empty', 0)} / {m.get('c_error', 0)}"


def _sum(methods: Dict[str, Any], key: str) -> int:
    return sum(int(m.get(key, 0) or 0) for m in methods.values())


def render_repo(data: Dict[str, Any]) -> List[str]:
    out: List[str] = []
    out.append(f"## {data.get('repo', 'n/a')}")
    out.append("")
    out.append("### Provenance")
    out.append("")
    out.append("| Field | Value |")
    out.append("|:---|:---|")
    for label, key in PROVENANCE_ROWS:
        if key is None and label.startswith("rust-analyzer canary"):
            val = f"{data.get('ra_canary_wait_ms', 'n/a')} / {data.get('ra_canary_attempts', 'n/a')}"
        elif key is None:
            val = (f"{data.get('ra_canary_pre_query_wait_ms', 'n/a')} / "
                   f"{data.get('ra_canary_pre_query_attempts', 'n/a')}")
        else:
            val = data.get(key, "n/a")
        out.append(f"| {label} | `{val}` |")
    out.append("")
    ridx = data["ridx"]
    ra = data["rust_analyzer"]
    out.append("### Totals")
    out.append("")
    out.append("| Server | Error responses | Failed scenarios | `-32603` | Peak RSS (MB) |")
    out.append("|:---|---:|---:|---:|---:|")
    for name, srv in (("RIDX", ridx), ("rust-analyzer", ra)):
        methods = srv["methods"]
        internal = sum(int(m.get("error_codes", {}).get("-32603", 0)) for m in methods.values())
        out.append(f"| {name} | {_sum(methods, 'errors')} | {_sum(methods, 'c_error')} | {internal} | "
                   f"{srv.get('peak_rss_mb', 0.0):.2f} |")
    out.append("")
    out.append("### Per method (a non-empty / b empty / c failed; warm p50 / p99 in ms)")
    out.append("")
    out.append("| # | Method | RIDX a/b/c | RIDX p50 | RIDX p99 | RIDX codes | RA a/b/c | RA p50 | RA p99 | RA codes |")
    out.append("|---:|:---|:---|---:|---:|:---|:---|---:|---:|:---|")
    for key, m in sorted(ridx["methods"].items(), key=lambda kv: kv[1]["id"]):
        a = ra["methods"].get(key, {})
        out.append(
            f"| {m['id']} | `{m['rpc_method']}` | {_abc(m)} | {_ms(m.get('warm_p50_us'))} | {_ms(m.get('warm_p99_us'))} | "
            f"{_codes(m.get('error_codes', {}))} | {_abc(a)} | {_ms(a.get('warm_p50_us'))} | {_ms(a.get('warm_p99_us'))} | "
            f"{_codes(a.get('error_codes', {}))} |"
        )
    out.append("")
    return out


def render(pub_dir: Path) -> str:
    out: List[str] = []
    out.append("# LSP Matrix Benchmark: 300 variations x 78 methods")
    out.append("")
    out.append("Generated by `bench/matrix_report.py` from `bench/published/matrix_<repo>_booster.json`. "
               "Methodology: `bench/BENCHMARK_METHODOLOGY.md`, section 5. Hardware: `booster` "
               "(AMD Ryzen 9 7950X, 16 cores / 32 threads); the runner records 125 GB of RAM as reported by "
               "the operating system for the 128 GB installed.")
    out.append("")
    published: List[str] = []
    missing: List[str] = []
    for key, canon in REPO_ORDER:
        f = pub_dir / f"matrix_{key}_booster.json"
        sidecar = pub_dir / f"matrix_{key}_booster.provenance.json"
        if f.is_file() and sidecar.is_file():
            published.append(key)
        else:
            missing.append(canon)
    out.append("## Status")
    out.append("")
    out.append(f"Published on the current harness: {', '.join(f'`{k}`' for k in published) or 'none'}. "
               f"Pending: {', '.join(f'`{c}`' for c in missing) or 'none'}.")
    out.append("")
    out.append("## Notes")
    out.append("")
    for n in NOTES:
        out.append(f"- {n}")
    out.append("")
    out.append("## Known harness defects (fixed, in commit order)")
    out.append("")
    for commit, text in KNOWN_HARNESS_DEFECTS:
        out.append(f"- `{commit}`: {text}")
    out.append("")
    for key in published:
        with open(pub_dir / f"matrix_{key}_booster.json", "r", encoding="utf-8") as fh:
            out.extend(render_repo(json.load(fh)))
    return "\n".join(out).rstrip() + "\n"


def main(argv: Optional[List[str]] = None) -> None:
    parser = argparse.ArgumentParser(description="Render docs/BENCHMARK_MATRIX.md from published matrix runs")
    parser.add_argument("--pub", default=str(Path(__file__).resolve().parent / "published"))
    parser.add_argument("--output", default=str(Path(__file__).resolve().parents[1] / "docs" / "BENCHMARK_MATRIX.md"))
    args = parser.parse_args(argv)
    text = render(Path(args.pub))
    Path(args.output).write_text(text, encoding="utf-8")
    print(f"Wrote {args.output} ({len(text.splitlines())} lines)")


if __name__ == "__main__":
    main()
