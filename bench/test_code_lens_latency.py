# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Unit tests for code_lens_latency: the bogus-lens checks, the per-run summary and the answer
comparison between binaries work on fixed rows, without a server.
"""

import sys
from pathlib import Path

bench_dir = Path(__file__).resolve().parent
if str(bench_dir) not in sys.path:
    sys.path.insert(0, str(bench_dir))

from code_lens_latency import compare_answers, digest, lens_checks, summarize


def _lens(line, name):
    return {"range": {"start": {"line": line, "character": 0}, "end": {"line": line, "character": 3}},
            "data": {"uri": "file:///r/src/lib.rs", "line": line + 1, "character": 1, "name": name}}


def _row(name, warm, lenses, error=None, dig="d"):
    return {"name": name, "file": f"src/{name}.rs", "bytes": 10, "lines": 3, "lenses": lenses,
            "lenses_beyond_eof": 0, "lens_names_missing": 0, "digest": dig, "error": error,
            "cold_us": warm * 2, "warm_us": warm}


def test_lens_checks_count_lenses_outside_the_file():
    facts = {"bytes": 30, "lines": 3, "text": "fn alpha() {}\nstruct Beta;\nfn gamma() {}\n"}
    result = [_lens(0, "alpha"), _lens(1, "Beta"), _lens(2, "gamma"), _lens(40, "delta"), _lens(41, "delta")]
    assert lens_checks(result, facts) == {"lenses_beyond_eof": 2, "lens_names_missing": 1}
    assert lens_checks(None, facts) == {"lenses_beyond_eof": None, "lens_names_missing": None}
    print("PASS: lens_checks counts lenses past the end of the file and names absent from it")


def test_summarize_percentiles_and_slowest():
    rows = [_row(f"f{i}", float(i + 1), i) for i in range(10)] + [_row("bad", 0.0, None, error="TIMEOUT")]
    s = summarize(rows)
    assert (s["files"], s["errors"], s["lenses_total"]) == (11, 1, 45)
    assert (s["warm_p50_us"], s["warm_p99_us"], s["cold_p50_us"]) == (5.5, 9.9, 11.0)
    assert [r["name"] for r in s["slowest_warm"]] == [f"f{i}" for i in range(9, -1, -1)]
    print("PASS: summarize reports percentiles over error-free rows and the slowest files")


def test_compare_answers_by_digest():
    runs = [
        {"name": "a", "rows": [_row("x", 1.0, 1, dig="same"), _row("y", 1.0, 1, dig="a-only")]},
        {"name": "b", "rows": [_row("x", 1.0, 1, dig="same"), _row("y", 1.0, 1, dig="b-only")]},
        {"name": "a", "rows": [_row("x", 1.0, 1, dig="same"), _row("y", 1.0, 1, dig="a-again")]},
    ]
    cmp = compare_answers(runs)
    assert cmp["pairs"] == [{"a": "a", "b": "b", "files": 2, "identical": 1, "differing": ["src/y.rs"]}]
    assert cmp["unstable_within_binary"] == {"a": ["src/y.rs"]}
    assert digest([_lens(0, "alpha")]) == digest([{"data": _lens(0, "alpha")["data"], "range": _lens(0, "alpha")["range"]}])
    print("PASS: compare_answers pairs binaries by digest and flags unstable answers")


if __name__ == "__main__":
    test_lens_checks_count_lenses_outside_the_file()
    test_summarize_percentiles_and_slowest()
    test_compare_answers_by_digest()
    print("PASS: all code_lens_latency tests passed")
