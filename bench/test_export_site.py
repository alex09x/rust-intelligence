# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Unit test for export_site: the site payload is built from the published matrix JSON only, keeps the
row shape app.js renders, and its notes carry no ratios or comparative wording.
"""

import json
import sys
import tempfile
from pathlib import Path

bench_dir = Path(__file__).resolve().parent
if str(bench_dir) not in sys.path:
    sys.path.insert(0, str(bench_dir))

from export_site import export_site_bench
from test_matrix_report import FORBIDDEN_MULTIPLIERS, FORBIDDEN_WORDS, _fixture


def test_export_from_published_json():
    with tempfile.TemporaryDirectory() as td:
        pub = Path(td) / "published"
        pub.mkdir()
        data = _fixture()
        data["total_methods"] = 78
        (pub / "matrix_ripgrep_booster.json").write_text(json.dumps(data), encoding="utf-8")
        (pub / "matrix_ripgrep_booster.provenance.json").write_text("{}", encoding="utf-8")
        out = Path(td) / "site" / "bench.json"
        payload = export_site_bench(pub, out)
        on_disk = json.loads(out.read_text(encoding="utf-8"))
        assert on_disk == payload
        again = Path(td) / "again" / "bench.json"
        export_site_bench(pub, again)
        assert again.read_bytes() == out.read_bytes(), "export is not deterministic"
        assert (again.parent / "bench.data.js").read_bytes() == (out.parent / "bench.data.js").read_bytes()
        js = (Path(td) / "site" / "bench.data.js").read_text(encoding="utf-8")
        assert js.splitlines()[1].startswith("window.RIDX_BENCH = {")
        assert json.loads(js.split("window.RIDX_BENCH = ", 1)[1].rstrip().rstrip(";")) == payload

        assert list(payload["workspaces"]) == ["ripgrep"]
        ws = payload["workspaces"]["ripgrep"]
        assert ws["meta"] == "ripgrep · 27 index shards · RIDX d3b7810 · booster"
        assert ws["target"] == "LSP matrix · 300 items per method · warm latencies · corpus 4649aa9"
        assert len(ws["rows"]) == 12
        assert ws["rows"][0] == ["Go to definition (warm p50)", 7.75e-05, "78 µs", 0.0001, "100 µs"]
        assert ws["rows"][1] == ["Go to definition (tail p99)", 0.0002313, "231 µs", 0.0002, "200 µs"]
        assert ws["rows"][4] == ["Hover (warm p50)", 0.0, "n/a", 0.0, "n/a"]
        assert ws["rows"][10] == ["Rename (warm p50)", 0.0002, "200 µs", 0.0003, "300 µs"]
        assert ws["memory"] == {"ridx_peak_rss_mb": 81.73, "ra_peak_rss_mb": 2176.41}
        assert ws["coverage"] == {"ridx_answered": 2, "ra_answered": 2, "request_methods": 38}
        assert payload["ridx_commit"] == "d3b7810f2504452dd66570969eef2def04028be5"
        assert payload["method_note"].startswith("1 repositories, 78 LSP methods, 300 sampled items per method (22,204 requests per server and repository, seed 42); RIDX d3b7810, driver 8884205b28dd2daa, rust-analyzer 1.98.1 (48a229c 2026-09-01); node booster, CPU set 24-31.")
        assert payload["limitations"][0].startswith("Outside mutation case B nothing in the matrix checks that an answer is correct")
        assert payload["limitations"][1] == "textDocument/codeLens is not counted: the matrix client declares neither textDocument.codeLens nor the rust-analyzer.* commands (experimental.commands) that rust-analyzer requires before it builds any lens, so rust-analyzer answered [] on every item (known harness defects in docs/BENCHMARK_MATRIX.md)."
        assert payload["limitations"][2] == "textDocument/content is not counted: the published runs were measured before harness commit d27ac0e fixed its parameters (known harness defects in docs/BENCHMARK_MATRIX.md)."
        assert "Peak RSS over the run: RIDX 82 to 82 MB, rust-analyzer 2176 to 2176 MB" in payload["limitations"][-2]
        assert payload["limitations"][-1] == "Requests cancelled on the client timeout: RIDX 0, rust-analyzer 0."
        for key in ("summary", "completion", "accuracy"):
            assert key not in payload and key not in ws
        prose = payload["method_note"] + " ".join(payload["limitations"])
        assert FORBIDDEN_WORDS.search(prose) is None, FORBIDDEN_WORDS.search(prose)
        assert FORBIDDEN_MULTIPLIERS.search(prose) is None, FORBIDDEN_MULTIPLIERS.search(prose)
        print("PASS: site payload is built from the matrix JSON with the row shape app.js renders")


if __name__ == "__main__":
    test_export_from_published_json()
    print("PASS: all export_site tests passed")
