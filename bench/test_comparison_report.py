# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Unit tests for comparison_report: numbers come from the published matrix JSON, the output is
deterministic, missing methods render as n/a, and the text carries no ratios or comparative wording.
"""

import json
import sys
import tempfile
from pathlib import Path

bench_dir = Path(__file__).resolve().parent
if str(bench_dir) not in sys.path:
    sys.path.insert(0, str(bench_dir))

from comparison_report import render
from test_matrix_report import FORBIDDEN_MULTIPLIERS, FORBIDDEN_WORDS, _fixture, _method


def _published(td):
    pub = Path(td)
    data = _fixture()
    data["ridx"]["peak_threads"], data["ridx"]["peak_fds"] = 3, 5
    data["rust_analyzer"]["peak_threads"], data["rust_analyzer"]["peak_fds"] = 53, 16
    data["ridx_cancelled_on_timeout"], data["ra_cancelled_on_timeout"] = 0, 1
    data["ridx"]["methods"]["code_lens"] = _method(59, "textDocument/codeLens", 295, 5, 0, 349.1, 17234.4, {})
    data["rust_analyzer"]["methods"]["code_lens"] = _method(59, "textDocument/codeLens", 0, 300, 0, 59.9, 2278.3, {})
    data["ridx"]["methods"]["moniker"] = _method(32, "textDocument/moniker", 0, 300, 0, 10.0, 20.0, {})
    data["rust_analyzer"]["methods"]["moniker"] = _method(32, "textDocument/moniker", 0, 0, 300, 10.0, 20.0, {"-32601": 300})
    (pub / "matrix_ripgrep_booster.json").write_text(json.dumps(data), encoding="utf-8")
    (pub / "matrix_ripgrep_booster.provenance.json").write_text("{}", encoding="utf-8")
    return pub


def test_render_from_published_json():
    with tempfile.TemporaryDirectory() as td:
        pub = _published(td)
        text = render(pub)
        assert text == render(pub), "render is not deterministic"
        lines = text.splitlines()
        assert "Published runs: `BurntSushi/ripgrep`. Pending: `tokio-rs/tokio`, `rust-lang/rust-analyzer`, `bevyengine/bevy`, `serde-rs/serde`." in lines
        assert "| RIDX commit | `d3b7810f2504452dd66570969eef2def04028be5` |" in lines
        assert "| Requests per server and repository | `22204` |" in lines
        assert "| BurntSushi/ripgrep | `4649aa9` | 27 | 81.73 | 2176.41 | 3 / 5 | 53 / 16 | 800.0 | 110.0 | 0 / 1 | 3 / 2 of 39 |" in lines
        assert "### BurntSushi/ripgrep (corpus commit `4649aa9`)" in lines
        assert "| Go to definition | 291 / 9 / 0 | 78 µs | 231 µs | 294 / 6 / 0 | 100 µs | 200 µs |" in lines
        assert "| Rename | 178 / 0 / 14 | 200 µs | 2.10 ms | 295 / 0 / 3 | 300 µs | 400 µs |" in lines
        assert "| Code lens | 295 / 5 / 0 | 349 µs | 17.23 ms | 0 / 300 / 0 | 60 µs | 2.28 ms |" in lines
        assert "| Hover | n/a | n/a | n/a | n/a | n/a | n/a |" in lines
        assert "- RIDX returned no non-empty answer on any item while rust-analyzer did: none." in lines
        assert "  - `textDocument/moniker` (BurntSushi/ripgrep)" in lines
        assert "  - `textDocument/codeLens` (BurntSushi/ripgrep)" in lines
        assert "- `textDocument/content` is not counted: the published runs were measured before harness commit `d27ac0e` fixed its parameters (known harness defects in `docs/BENCHMARK_MATRIX.md`)." in lines
        assert any("Outside mutation case B nothing in the matrix checks that an answer is correct" in l for l in lines)
        assert "- Completion recall and precision reruns against rust-analyzer: 0 `rerun_completion_*.jsonl` and 0 `completion_regression_*.json` files. Tooling (`run_completion_benchmark.py`, `verify_completion_regression.py`, `validate_scenarios.py`) exists up to commit `9a75047`." in lines
        assert FORBIDDEN_WORDS.search(text) is None, FORBIDDEN_WORDS.search(text)
        assert FORBIDDEN_MULTIPLIERS.search(text) is None, FORBIDDEN_MULTIPLIERS.search(text)
        print("PASS: comparison report renders setup, glance, headline and coverage sections from JSON")


def test_render_without_published_runs():
    with tempfile.TemporaryDirectory() as td:
        text = render(Path(td))
        assert "Published runs: none. Pending: `BurntSushi/ripgrep`, `tokio-rs/tokio`, `rust-lang/rust-analyzer`, `bevyengine/bevy`, `serde-rs/serde`." in text.splitlines()
        assert "## 1. Setup" not in text
        assert "## 5. Earlier publications" in text
        print("PASS: comparison report without runs lists every repository as pending")


if __name__ == "__main__":
    test_render_from_published_json()
    test_render_without_published_runs()
    print("PASS: all comparison_report tests passed")
