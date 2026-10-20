# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Unit test for matrix_report: numbers come from the published JSON, output is deterministic,
and the report carries no comparative marketing wording.
"""

import json
import re
import sys
import tempfile
from pathlib import Path

bench_dir = Path(__file__).resolve().parent
if str(bench_dir) not in sys.path:
    sys.path.insert(0, str(bench_dir))

from matrix_report import render

FORBIDDEN_WORDS = re.compile(r"\b(faster|slower|speedup|beat|beats)\b", re.IGNORECASE)
FORBIDDEN_MULTIPLIERS = re.compile(r"(?<![\w.])\d+(\.\d+)?x\b|\\times")


def _method(mid, rpc, a, b, c, p50, p99, codes):
    return {
        "id": mid, "domain": "Language Intelligence", "rpc_method": rpc, "tests_run": a + b + c,
        "errors": sum(codes.values()), "error_codes": codes, "a_non_empty": a, "b_empty": b, "c_error": c,
        "warm_p50_us": p50, "warm_p99_us": p99,
    }


def _fixture():
    return {
        "repo": "BurntSushi/ripgrep", "repo_commit": "4649aa9700619f94cf9c66876e9549d83420e16c",
        "ridx_commit": "d3b7810f2504452dd66570969eef2def04028be5", "ridx_binary_dirty": False,
        "driver_hash": "8884205b28dd2daa", "driver_build_commit": "cdf975d9dc93a314436e40188bf5e25d09144ccf",
        "rustc": "rustc 1.98.1 (48a229cea 2026-09-01)", "ra_version": "rust-analyzer 1.98.1 (48a229c 2026-09-01)",
        "node": "booster (AMD Ryzen 9 7950X 16-Core Processor, 32 threads, 125 GB RAM)", "taskset": "24-31",
        "seed": 42, "sample_count_per_method": 300, "total_tests_per_server": 22204, "sysroot_cas_key": "5368ac0ba3a3ce1e",
        "ridx_index_shards": 27, "timestamp_start": "2026-10-13T03:40:00+00:00", "timestamp_end": "2026-10-13T03:55:00+00:00",
        "ridx_duration_sec": 800.0, "ra_duration_sec": 110.0, "ra_initial_wait_ms": 2500.0,
        "ra_canary_wait_ms": 1300.0, "ra_canary_attempts": 2,
        "ra_canary_pre_query_wait_ms": 1050.0, "ra_canary_pre_query_attempts": 2,
        "ridx": {"peak_rss_mb": 81.73, "methods": {
            "initialize": {"id": 1, "domain": "Lifecycle", "rpc_method": "initialize", "tests_run": 1, "errors": 0, "error_codes": {}, "warm_p50_us": 65298.0, "warm_p99_us": 65298.0},
            "definition": _method(15, "textDocument/definition", 291, 9, 0, 77.5, 231.3, {}),
            "completion_resolve": _method(26, "completionItem/resolve", 300, 0, 0, 100.0, 900.0, {}),
            "rename": _method(49, "textDocument/rename", 178, 0, 14, 200.0, 2100.0, {"-32803": 42}),
        }},
        "rust_analyzer": {"peak_rss_mb": 2176.41, "methods": {
            "initialize": {"id": 1, "domain": "Lifecycle", "rpc_method": "initialize", "tests_run": 1, "errors": 0, "error_codes": {}, "warm_p50_us": 1000.0, "warm_p99_us": 1000.0},
            "definition": _method(15, "textDocument/definition", 294, 6, 0, 100.0, 200.0, {}),
            "completion_resolve": _method(26, "completionItem/resolve", 300, 0, 0, 1200.0, 4000.0, {}),
            "rename": _method(49, "textDocument/rename", 295, 0, 3, 300.0, 400.0, {"-32602": 9, "-32603": 3}),
        }},
    }


def test_render_from_published_json():
    with tempfile.TemporaryDirectory() as td:
        pub = Path(td)
        (pub / "matrix_ripgrep_booster.json").write_text(json.dumps(_fixture()), encoding="utf-8")
        (pub / "matrix_ripgrep_booster.provenance.json").write_text("{}", encoding="utf-8")
        text_a = render(pub)
        text_b = render(pub)
        assert text_a == text_b, "render is not deterministic"
        lines = text_a.splitlines()
        assert "## BurntSushi/ripgrep" in lines
        assert "Published on the current harness: `ripgrep`. Pending: `tokio-rs/tokio`, `rust-lang/rust-analyzer`, `bevyengine/bevy`, `serde-rs/serde`." in lines
        assert "| RIDX commit | `d3b7810f2504452dd66570969eef2def04028be5` |" in lines
        assert "| rust-analyzer canary wait (ms) / attempts | `1300.0 / 2` |" in lines
        assert "| rust-analyzer pre-query canary wait (ms) / attempts | `1050.0 / 2` |" in lines
        assert "| RIDX | 42 | 14 | 0 | 81.73 |" in lines
        assert "| rust-analyzer | 12 | 3 | 3 | 2176.41 |" in lines
        assert "| 1 | `initialize` | 1 run | 65.3 | 65.3 | - | 1 run | 1.0 | 1.0 | - |" in lines
        assert "| 15 | `textDocument/definition` | 291 / 9 / 0 | 0.1 | 0.2 | - | 294 / 6 / 0 | 0.1 | 0.2 | - |" in lines
        assert "| 26 | `completionItem/resolve` | n/a (synthetic item) | 0.1 | 0.9 | - | n/a (synthetic item) | 1.2 | 4.0 | - |" in lines
        assert "| 49 | `textDocument/rename` | 178 / 0 / 14 | 0.2 | 2.1 | -32803: 42 | 295 / 0 / 3 | 0.3 | 0.4 | -32602: 9, -32603: 3 |" in lines
        assert "- Rows marked `n/a (synthetic item)` send an item built by the harness, not one returned by the server: the seven `*/resolve` methods, `workspace/executeCommand`, and the call and type hierarchy methods whose items are not taken from a `prepare*` answer. A non-empty answer there is usually the item echoed back, so only latency and error codes are shown; the a/b/c counters stay in the published JSON." in lines
        assert '- `d27ac0e`: `textDocument/content` was sent as `{"textDocument": {"uri"}}` instead of `{"uri"}`; RIDX read an empty uri and answered `{"text": ""}`, counted as non-empty for all 300 requests, so row 75 of the runs measured before this commit is not a measurement of the method (rust-analyzer answers `-32601`: it does not implement the method). The row now sends `{"uri"}`.' in lines
        assert FORBIDDEN_WORDS.search(text_a) is None, FORBIDDEN_WORDS.search(text_a)
        assert FORBIDDEN_MULTIPLIERS.search(text_a) is None, FORBIDDEN_MULTIPLIERS.search(text_a)
        assert "- `d3b7810`: the synchronization block (ids 8-14) makes rust-analyzer reload the workspace and answer `textDocument/definition` with empty results for a few seconds (measured on tokio: 229/300 non-empty before the block, 0/300 right after, 229/300 sixty seconds later); the runner now re-establishes readiness before the first query method." in lines
        print("PASS: matrix report renders provenance, totals and per-method rows from JSON, deterministically")


def test_render_without_published_runs():
    with tempfile.TemporaryDirectory() as td:
        text = render(Path(td))
        assert "Published on the current harness: none. Pending: `BurntSushi/ripgrep`, `tokio-rs/tokio`, `rust-lang/rust-analyzer`, `bevyengine/bevy`, `serde-rs/serde`." in text.splitlines()
        assert "## BurntSushi/ripgrep" not in text
        print("PASS: matrix report with no published runs lists every repository as pending")


if __name__ == "__main__":
    test_render_from_published_json()
    test_render_without_published_runs()
    print("PASS: all matrix_report tests passed")
