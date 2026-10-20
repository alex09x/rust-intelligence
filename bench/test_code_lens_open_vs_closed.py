# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Unit tests for code_lens_open_vs_closed: the version parsing, the multiset comparison of the two
answers, the warm percentiles and the comparison between binaries work on fixed data, without a
server.
"""

import sys
from pathlib import Path

bench_dir = Path(__file__).resolve().parent
if str(bench_dir) not in sys.path:
    sys.path.insert(0, str(bench_dir))

from collections import Counter

from code_lens_latency import lens_key
from code_lens_open_vs_closed import compare_binaries, compare_multisets, parse_version, side_stats, summarize


def _lens(line, name, character=4):
    return {"range": {"start": {"line": line, "character": character}, "end": {"line": line, "character": character + len(name)}},
            "data": {"uri": "file:///r/src/lib.rs", "line": line + 1, "character": character + 1, "name": name}}


def _keys(*lenses):
    return Counter(lens_key(lens) for lens in lenses)


def test_parse_version_reads_commit_and_dirty_flag():
    assert parse_version("ridx 0.1.0 (commit: 8cb4341bcc543550cf34866dbf71edae6ee04dac, dirty: false)") == {
        "version": "ridx 0.1.0 (commit: 8cb4341bcc543550cf34866dbf71edae6ee04dac, dirty: false)",
        "commit": "8cb4341bcc543550cf34866dbf71edae6ee04dac", "dirty": False}
    assert parse_version("ridx 0.1.0 (commit: abc, dirty: true)")["dirty"] is True
    assert parse_version("unavailable: no such file") == {"version": "unavailable: no such file", "commit": None, "dirty": None}
    print("PASS: parse_version reads the commit and the dirty flag")


def test_compare_multisets_identical_and_differing():
    alpha, beta, x = _lens(0, "alpha"), _lens(5, "beta"), _lens(0, "x", character=13)
    same = compare_multisets(_keys(alpha, beta), _keys(beta, alpha))
    assert (same["identical"], same["common"], same["only_closed"], same["only_open"]) == (True, 2, 0, 0)
    # The closed answer has a lens on a variant the open side lacks; the open side has one on a
    # parameter, and a lens of `beta` twice where the closed side has it once.
    red = _lens(6, "Red")
    cmp = compare_multisets(_keys(alpha, beta, red), _keys(alpha, beta, beta, x))
    assert (cmp["closed_lenses"], cmp["open_lenses"], cmp["common"]) == (3, 4, 2)
    assert (cmp["identical"], cmp["only_closed"], cmp["only_open"]) == (False, 1, 2)
    assert cmp["only_closed_names_top"] == [("Red", 1)]
    assert sorted(cmp["only_open_names_top"]) == [("beta", 1), ("x", 1)]
    print("PASS: compare_multisets reports common, only-closed and only-open lenses with their names")


def test_side_stats_and_summarize_aggregate_warm_samples():
    assert side_stats([]) == {"n": 0, "warm_p50_us": None, "warm_p99_us": None}
    assert side_stats([float(i) for i in range(1, 11)]) == {"n": 10, "warm_p50_us": 5.5, "warm_p99_us": 9.9}
    rows = [
        {"error": None, "identical": True, "closed_lenses": 3, "open_lenses": 3},
        {"error": None, "identical": False, "closed_lenses": 5, "open_lenses": 2},
        {"error": "TIMEOUT", "identical": False, "closed_lenses": 0, "open_lenses": 0},
    ]
    s = summarize(rows, [1.0, 2.0, 3.0, 4.0], [10.0, 20.0])
    assert (s["files"], s["errors"], s["identical_files"]) == (3, 1, 1)
    assert (s["closed_lenses"], s["open_lenses"]) == (8, 5)
    assert (s["closed"]["warm_p50_us"], s["open"]["warm_p50_us"], s["open"]["n"]) == (2.5, 15.0, 2)
    print("PASS: summarize counts identical files and aggregates the warm samples of the corpus")


def test_compare_binaries_pairs_open_answers_and_the_index_composition():
    alpha, beta, x = _lens(0, "alpha"), _lens(5, "beta"), _lens(0, "x", character=13)
    closed = _keys(alpha, beta)
    answers = {
        # The old binary: the open answer differs from the closed one on both files.
        "base": {"src/a.rs": (closed, _keys(alpha, beta, x)), "src/b.rs": (closed, _keys(x))},
        # The new binary: the open answer is the closed answer on a.rs, still differs on b.rs.
        "fix": {"src/a.rs": (closed, closed), "src/b.rs": (closed, _keys(alpha))},
    }
    cmp = compare_binaries(answers)
    assert cmp["binaries"] == ["base", "fix"]
    assert cmp["pairs"] == [{"a": "base", "b": "fix", "files": 2, "open_differing": ["src/a.rs", "src/b.rs"],
                             "open_b_equals_closed_a": 1}]
    print("PASS: compare_binaries lists files whose open answers differ and counts the carried-over composition")


if __name__ == "__main__":
    test_parse_version_reads_commit_and_dirty_flag()
    test_compare_multisets_identical_and_differing()
    test_side_stats_and_summarize_aggregate_warm_samples()
    test_compare_binaries_pairs_open_answers_and_the_index_composition()
    print("PASS: all code_lens_open_vs_closed tests passed")
