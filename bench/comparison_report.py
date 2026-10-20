# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Renders LSP_BENCHMARK_COMPARISON_REPORT.md from the published matrix runs. Every number is read
from bench/published/matrix_<repo>_booster.json; the report puts latencies and answer counters of
the two servers side by side and computes no ratios. The full 78-method tables live in
docs/BENCHMARK_MATRIX.md (bench/matrix_report.py).
"""

import argparse
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

from matrix_headline import (CANONICAL, HARNESS_DEFECT_ROWS, HEADLINE_METHODS, REQUEST_KEYS, abc,
                             answered, by_method, defect_notes, distinct, fmt_us, load_published,
                             ra_not_implemented, ra_silent, ridx_silent, short, where)

Runs = List[Tuple[str, Dict[str, Any]]]

SETUP_FIELDS = [
    ("RIDX commit", "ridx_commit"), ("Driver hash", "driver_hash"),
    ("Driver build commit", "driver_build_commit"), ("rustc", "rustc"),
    ("rust-analyzer", "ra_version"), ("Node", "node"), ("CPU set", "taskset"), ("Seed", "seed"),
    ("Methods", "total_methods"), ("Items per method", "sample_count_per_method"),
    ("Requests per server and repository", "total_tests_per_server"),
]

PILOT_TOOLS = "`pilot_runner.py`, `worktree_bench.py`, `corpus.py`, `sampling.py`, `report.py`"
COMPLETION_TOOLS = "`run_completion_benchmark.py`, `verify_completion_regression.py`, `validate_scenarios.py`"
# The last commit that still contains the tooling of the earlier pipelines (the parent of the commit
# that removed it); `git checkout <commit> -- bench/<file>` restores a script.
TOOLING_LAST_COMMIT = "9a75047"


def _row(cells: List[str]) -> str:
    return "| " + " | ".join(cells) + " |"


def render_setup(runs: Runs) -> List[str]:
    out = ["## 1. Setup", "",
           "Values shared by the five runs; a cell lists every distinct value when the runs differ.", "",
           "| Field | Value |", "|:---|:---|"]
    for label, field in SETUP_FIELDS:
        out.append(_row([label, ", ".join(f"`{v}`" for v in distinct(runs, field))]))
    out.append("")
    return out


def render_glance(runs: Runs) -> List[str]:
    out = ["## 2. The five repositories at a glance", "",
           f"Request methods are the {len(REQUEST_KEYS)} rows whose items come from the corpus "
           "(see section 4). Peak RSS, threads and file descriptors are process telemetry "
           "sampled during the run. Cancelled requests hit the client timeout.", "",
           _row(["Repository", "Corpus commit", "Index shards", "RIDX peak RSS (MB)",
                 "rust-analyzer peak RSS (MB)", "RIDX threads / fds", "rust-analyzer threads / fds",
                 "RIDX suite (s)", "rust-analyzer suite (s)", "Cancelled RIDX / rust-analyzer",
                 "Request methods answered RIDX / rust-analyzer"]),
           "|:---|:---|---:|---:|---:|:---|:---|---:|---:|:---|:---|"]
    for key, data in runs:
        ridx, ra = data["ridx"], data["rust_analyzer"]
        out.append(_row([
            CANONICAL.get(key, key), f"`{short(data.get('repo_commit'))}`",
            str(data.get("ridx_index_shards", "n/a")),
            f"{ridx.get('peak_rss_mb', 0.0):.2f}", f"{ra.get('peak_rss_mb', 0.0):.2f}",
            f"{ridx.get('peak_threads', 'n/a')} / {ridx.get('peak_fds', 'n/a')}",
            f"{ra.get('peak_threads', 'n/a')} / {ra.get('peak_fds', 'n/a')}",
            f"{data.get('ridx_duration_sec', 0.0):.1f}", f"{data.get('ra_duration_sec', 0.0):.1f}",
            f"{data.get('ridx_cancelled_on_timeout', 0)} / {data.get('ra_cancelled_on_timeout', 0)}",
            f"{answered(ridx['methods'])} / {answered(ra['methods'])} of {len(REQUEST_KEYS)}",
        ]))
    out.append("")
    return out


def render_headlines(runs: Runs) -> List[str]:
    out = ["## 3. Headline methods per repository", "",
           "`a / b / c` = non-empty, empty and failed answers out of the sampled items; latencies are "
           "the warm p50 and p99 over the items (methodology, sections 5 and 6). The remaining "
           "methods are in `docs/BENCHMARK_MATRIX.md`; rows excluded as harness defects (section 4) "
           "are not shown.", ""]
    for key, data in runs:
        ridx, ra = data["ridx"]["methods"], data["rust_analyzer"]["methods"]
        out += [f"### {CANONICAL.get(key, key)} (corpus commit `{short(data.get('repo_commit'))}`)", "",
                _row(["Method", "RIDX a / b / c", "RIDX p50", "RIDX p99", "rust-analyzer a / b / c",
                      "rust-analyzer p50", "rust-analyzer p99"]),
                "|:---|:---|---:|---:|:---|---:|---:|"]
        for mkey, label in HEADLINE_METHODS:
            if mkey in HARNESS_DEFECT_ROWS:
                continue
            m, a = ridx.get(mkey, {}), ra.get(mkey, {})
            out.append(_row([label, abc(m), fmt_us(m.get("warm_p50_us")), fmt_us(m.get("warm_p99_us")),
                             abc(a), fmt_us(a.get("warm_p50_us")), fmt_us(a.get("warm_p99_us"))]))
        out.append("")
    return out


def _coverage_list(title: str, mapping: Dict[str, List[str]], total: int) -> List[str]:
    if not mapping:
        return [f"- {title}: none."]
    return [f"- {title}:"] + [f"  - `{method}` ({where(repos, total)})" for method, repos in mapping.items()]


def render_coverage(runs: Runs) -> List[str]:
    total = len(runs)
    out = ["## 4. Coverage differences", "",
           f"Over the {len(REQUEST_KEYS)} request methods whose items come from the corpus. The twelve "
           "rows that send an item built by the harness (the `*/resolve` methods, "
           "`workspace/executeCommand`, the call and type hierarchy methods), the notifications, the "
           "lifecycle and the mutation rows are not counted. Outside mutation case B nothing in the "
           "matrix checks that an answer is correct: `a / b / c` counts answers, and correctness is "
           "covered by the test suite and the fidelity scripts (methodology, sections 5, 7 and 9).", ""]
    out += _coverage_list("RIDX returned no non-empty answer on any item while rust-analyzer did",
                          by_method(runs, ridx_silent), total)
    out += _coverage_list("rust-analyzer answered `-32601` (method not found) on every item",
                          by_method(runs, ra_not_implemented), total)
    out += _coverage_list("rust-analyzer returned no non-empty answer on any item while RIDX did",
                          by_method(runs, ra_silent), total)
    out += [f"- {note}" for note in defect_notes(code=True)]
    out.append("")
    return out


def render_history(pub_dir: Path) -> List[str]:
    reruns = len(list(pub_dir.glob("rerun_completion_*.jsonl")))
    regressions = len(list(pub_dir.glob("completion_regression_*.json")))
    hover = ", ".join(f"`{p.name}`" for p in sorted(pub_dir.glob("rerun_hover_*.jsonl"))) or "none"
    fidelity = len(list(pub_dir.glob("selection_range_fidelity_*.json")))
    return [
        "## 5. Earlier publications", "",
        "Published data is never rewritten; the files below stay in `bench/published/` and the "
        "code that produced them is in the git history.", "",
        "- Five-repository pilot (strict-match scenarios against rust-analyzer, worktree and "
        "mutation cases): `pilot_5repos_2026-10-08.jsonl` with its `.provenance.json`. Tooling "
        f"({PILOT_TOOLS}) exists up to commit `{TOOLING_LAST_COMMIT}`: "
        f"`git checkout {TOOLING_LAST_COMMIT} -- bench/<file>` restores it (`bench/README.md` lists the "
        "commands). Earlier versions of this report were generated from the pilot.",
        f"- Completion recall and precision reruns against rust-analyzer: {reruns} "
        f"`rerun_completion_*.jsonl` and {regressions} `completion_regression_*.json` files. Tooling "
        f"({COMPLETION_TOOLS}) exists up to commit `{TOOLING_LAST_COMMIT}`.",
        f"- Hover rerun after the type-table binary search: {hover}.",
        "- Rename oracle on tokio (`cargo check --all-features` after the generated `WorkspaceEdit`): "
        "`rename_oracle.json`.",
        f"- selectionRange fidelity against rust-analyzer: {fidelity} "
        "`selection_range_fidelity_<repo>_ram9.json` files from `bench/selection_range_fidelity.py` "
        "(methodology, section 9).",
        "",
    ]


def render(pub_dir: Path) -> str:
    runs = load_published(pub_dir)
    out = ["# RIDX vs rust-analyzer: LSP Matrix Benchmark", "",
           "Generated by `bench/comparison_report.py` from `bench/published/matrix_<repo>_booster.json`; "
           "every number below is read from those files. Full 78-method tables per repository, "
           "provenance and the list of known harness defects: `docs/BENCHMARK_MATRIX.md`. Sampling, "
           "parameters and the `a / b / c` classification: `bench/BENCHMARK_METHODOLOGY.md`, section 5.",
           ""]
    published = ", ".join(f"`{CANONICAL.get(k, k)}`" for k, _ in runs) or "none"
    missing = ", ".join(f"`{c}`" for k, c in CANONICAL.items() if k not in dict(runs)) or "none"
    out += [f"Published runs: {published}. Pending: {missing}.", ""]
    if runs:
        out += render_setup(runs) + render_glance(runs) + render_headlines(runs) + render_coverage(runs)
    out += render_history(pub_dir)
    return "\n".join(out).rstrip() + "\n"


def main(argv: Optional[List[str]] = None) -> None:
    parser = argparse.ArgumentParser(description="Render LSP_BENCHMARK_COMPARISON_REPORT.md from published matrix runs")
    parser.add_argument("--pub", default=str(Path(__file__).resolve().parent / "published"))
    parser.add_argument("--output", default=str(Path(__file__).resolve().parents[1] / "LSP_BENCHMARK_COMPARISON_REPORT.md"))
    args = parser.parse_args(argv)
    text = render(Path(args.pub))
    Path(args.output).write_text(text, encoding="utf-8")
    print(f"Wrote {args.output} ({len(text.splitlines())} lines)")


if __name__ == "__main__":
    main()
