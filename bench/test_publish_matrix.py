# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Unit test for publish_matrix: provenance sidecar from runner fields, sanitization, determinism.
"""

import json
import sys
import tempfile
from pathlib import Path

bench_dir = Path(__file__).resolve().parent
if str(bench_dir) not in sys.path:
    sys.path.insert(0, str(bench_dir))

from publish_results import publish_matrix


def _raw_matrix(repo_label):
    return {
        "repo": repo_label,
        "repo_commit": "4649aa9700619f94cf9c66876e9549d83420e16c",
        "corpus_lockfile_sha256": "0" * 64,
        "ridx_commit": "d3b7810f2504452dd66570969eef2def04028be5",
        "ridx_binary_dirty": False,
        "driver_hash": "8884205b28dd2daa",
        "driver_build_commit": "cdf975d9dc93a314436e40188bf5e25d09144ccf",
        "driver_dirty": False,
        "driver_source": "index_manifest",
        "driver_newer_than_index": False,
        "index_built_at": "2026-10-12T07:55:46+00:00",
        "rustc": "rustc 1.98.1 (48a229cea 2026-09-01)",
        "ra_version": "rust-analyzer 1.98.1 (48a229c 2026-09-01)",
        "node": "booster (AMD Ryzen 9 7950X 16-Core Processor, 32 threads, 125 GB RAM)",
        "taskset": "24-31",
        "seed": 42,
        "sample_count_per_method": 300,
        "total_methods": 78,
        "total_tests_per_server": 22204,
        "sysroot_cas_key": "5368ac0ba3a3ce1e",
        "sysroot_transplanted": False,
        "ridx_index_shards": 27,
        "timestamp_start": "2026-10-13T03:40:00.000000+00:00",
        "timestamp_end": "2026-10-13T03:55:00.000000+00:00",
        "ridx_duration_sec": 800.0,
        "ra_duration_sec": 110.0,
        "ra_initial_wait_ms": 2500.0,
        "ra_canary_wait_ms": 1300.0,
        "ra_canary_attempts": 2,
        "ra_quiescent_pre_query_wait_ms": 2100.0,
        "ra_canary_pre_query_wait_ms": 1050.0,
        "ra_canary_pre_query_attempts": 2,
        "ridx": {"methods": {"definition": {"id": 15, "a_non_empty": 291}}},
        "rust_analyzer": {"methods": {"definition": {"id": 15, "a_non_empty": 294}}},
        "note": "/tmp/mbench/ripgrep/crates/core/main.rs",
    }


def test_publish_matrix_sidecar_sanitization_and_determinism():
    with tempfile.TemporaryDirectory() as td:
        src = Path(td) / "results"
        src.mkdir()
        (src / "matrix_ripgrep_booster.json").write_text(json.dumps(_raw_matrix("ripgrep"), indent=2), encoding="utf-8")
        positions = {"seed": 42, "sample_count": 300, "positions": {"definition": [
            {"name": "definition_0", "file": "/tmp/mbench/ripgrep/crates/core/main.rs", "line": 3, "character": 4, "symbol": "x"}
        ]}}
        (src / "matrix_ripgrep_booster.positions.json").write_text(json.dumps(positions, indent=2), encoding="utf-8")

        pub_a = Path(td) / "pub_a"
        pub_b = Path(td) / "pub_b"
        pub_a.mkdir()
        pub_b.mkdir()
        assert publish_matrix(src, pub_a, "ripgrep", "BurntSushi/ripgrep") is True
        assert publish_matrix(src, pub_b, "ripgrep", "BurntSushi/ripgrep") is True

        for name in ("matrix_ripgrep_booster.json", "matrix_ripgrep_booster.positions.json", "matrix_ripgrep_booster.provenance.json"):
            assert (pub_a / name).read_bytes() == (pub_b / name).read_bytes(), f"{name} is not byte-identical across runs"

        published = json.loads((pub_a / "matrix_ripgrep_booster.json").read_text(encoding="utf-8"))
        assert published["repo"] == "BurntSushi/ripgrep"
        assert published["note"] == "BurntSushi/ripgrep/crates/core/main.rs"
        published_pos = json.loads((pub_a / "matrix_ripgrep_booster.positions.json").read_text(encoding="utf-8"))
        assert published_pos["positions"]["definition"][0]["file"] == "BurntSushi/ripgrep/crates/core/main.rs"

        sidecar = json.loads((pub_a / "matrix_ripgrep_booster.provenance.json").read_text(encoding="utf-8"))
        assert sidecar["dataset"] == "matrix_ripgrep_booster"
        assert sidecar["repo"] == "BurntSushi/ripgrep"
        assert sidecar["ridx_commit"] == "d3b7810f2504452dd66570969eef2def04028be5"
        assert sidecar["harness_commit"] == "d3b7810f2504452dd66570969eef2def04028be5"
        assert sidecar["driver_hash"] == "8884205b28dd2daa"
        assert sidecar["corpus_lockfile_sha256"] == "0" * 64
        assert sidecar["driver_source"] == "index_manifest"
        assert sidecar["driver_newer_than_index"] is False
        assert sidecar["index_built_at"] == "2026-10-12T07:55:46+00:00"
        assert sidecar["ridx_index_shards"] == 27
        assert sidecar["ra_canary_pre_query_attempts"] == 2
        assert sidecar["date"] == "2026-10-13"
        assert sidecar["date_source"] == "timestamp_start recorded by the matrix runner"
        assert sidecar["positions_file"] == "matrix_ripgrep_booster.positions.json"
        assert len(sidecar["source_sha256"]) == 64
        assert len(sidecar["positions_source_sha256"]) == 64
        assert sidecar["sanitization"] == [
            {"rule": "repo -> BurntSushi/ripgrep", "replacements": 1},
            {"rule": "/tmp/<build-dir>/ripgrep/ -> BurntSushi/ripgrep/", "replacements": 1},
            {"rule": "positions: /tmp/<build-dir>/ripgrep/ -> BurntSushi/ripgrep/", "replacements": 1},
        ], sidecar["sanitization"]
        print("PASS: publish_matrix sidecar, sanitization and byte-for-byte determinism verified")


def test_publish_matrix_missing_source_is_skipped():
    with tempfile.TemporaryDirectory() as td:
        src = Path(td) / "results"
        src.mkdir()
        pub = Path(td) / "pub"
        pub.mkdir()
        assert publish_matrix(src, pub, "tokio", "tokio-rs/tokio") is False
        assert sorted(p.name for p in pub.iterdir()) == []
        print("PASS: publish_matrix skips a missing raw file without writing anything")


if __name__ == "__main__":
    test_publish_matrix_sidecar_sanitization_and_determinism()
    test_publish_matrix_missing_source_is_skipped()
    print("PASS: all publish_matrix tests passed")
