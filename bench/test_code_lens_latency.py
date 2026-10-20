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

from collections import Counter

from code_lens_latency import compare_answers, digest, lens_checks, lens_key, summarize


def _lens(line, name):
    return {"range": {"start": {"line": line, "character": 0}, "end": {"line": line, "character": 3}},
            "data": {"uri": "file:///r/src/lib.rs", "line": line + 1, "character": 1, "name": name}}


def _row(name, warm, lenses, error=None, dig="d"):
    return {"name": name, "file": f"src/{name}.rs", "bytes": 10, "lines": 3, "lenses": lenses,
            "lenses_past_eof": 0, "lens_names_missing": 0, "digest": dig, "error": error,
            "cold_us": warm * 2, "warm_us": warm}


def test_lens_checks_count_lenses_outside_the_file():
    facts = {"bytes": 30, "lines": 3, "text": "fn alpha() {}\nstruct Beta;\nfn gamma() {}\n"}
    # Lines 0..2 exist; a lens starting on line 3 is already past the end of a three-line file.
    result = [_lens(0, "alpha"), _lens(1, "Beta"), _lens(2, "gamma"), _lens(3, "delta"), _lens(41, "delta")]
    assert lens_checks(result, facts) == {"lenses_past_eof": 2, "lens_names_missing": 1}
    assert lens_checks(None, facts) == {"lenses_past_eof": None, "lens_names_missing": None}
    print("PASS: lens_checks counts lenses past the end of the file and names absent from it")


def test_summarize_percentiles_and_slowest():
    rows = [_row(f"f{i}", float(i + 1), i) for i in range(10)] + [_row("bad", 0.0, None, error="TIMEOUT")]
    s = summarize(rows)
    assert (s["files"], s["errors"], s["lenses_total"]) == (11, 1, 45)
    assert (s["warm_p50_us"], s["warm_p99_us"], s["cold_p50_us"]) == (5.5, 9.9, 11.0)
    assert [r["name"] for r in s["slowest_warm"]] == [f"f{i}" for i in range(9, -1, -1)]
    print("PASS: summarize reports percentiles over error-free rows and the slowest files")


def test_compare_answers_by_digest_and_multiset():
    runs = [
        {"name": "a", "round": 1, "rows": [_row("x", 1.0, 1, dig="same"), _row("y", 1.0, 3, dig="a-only")]},
        {"name": "b", "round": 1, "rows": [_row("x", 1.0, 1, dig="same"), _row("y", 1.0, 1, dig="b-only")]},
        {"name": "a", "round": 2, "rows": [_row("x", 1.0, 1, dig="same"), _row("y", 1.0, 3, dig="a-again")]},
    ]
    alpha, beta, far = lens_key(_lens(0, "alpha")), lens_key(_lens(1, "beta")), lens_key(_lens(40, "far"))
    answers = {"a": {"src/x.rs": Counter([alpha]), "src/y.rs": Counter([alpha, beta, far])},
               "b": {"src/x.rs": Counter([alpha]), "src/y.rs": Counter([beta])}}
    cmp = compare_answers(runs, answers)
    assert cmp["pairs"] == [{
        "a": "a", "b": "b", "files": 2, "identical": 1, "differing": 1, "subset_violations": 0,
        "b_past_eof_remaining": 0,
        "differing_details": [{"file": "src/y.rs", "lenses_a": 3, "lenses_b": 1, "removed": 2, "added": 0,
                               "b_subset_of_a": True, "b_lenses_past_eof": 0}],
    }]
    assert cmp["unstable_within_binary"] == {"a": ["src/y.rs"]}
    answers["b"]["src/y.rs"] = Counter([beta, lens_key(_lens(5, "new"))])
    assert compare_answers(runs, answers)["pairs"][0]["subset_violations"] == 1
    assert digest([_lens(0, "alpha")]) == digest([{"data": _lens(0, "alpha")["data"], "range": _lens(0, "alpha")["range"]}])
    print("PASS: compare_answers pairs binaries by digest and flags unstable answers")


if __name__ == "__main__":
    test_lens_checks_count_lenses_outside_the_file()
    test_summarize_percentiles_and_slowest()
    test_compare_answers_by_digest_and_multiset()
    print("PASS: all code_lens_latency tests passed")
