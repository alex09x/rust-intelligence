# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Unit test verifying RIDX index precondition verification in benchmark harness.
"""

import json
import sys
import tempfile
from pathlib import Path

bench_dir = Path(__file__).resolve().parent
if str(bench_dir) not in sys.path:
    sys.path.insert(0, str(bench_dir))

from matrix_provenance import check_ridx_index, count_index_shards


def test_index_precondition():
    with tempfile.TemporaryDirectory() as td:
        repo_dir = Path(td)
        # Test 1: Empty repo without index - expect rejection
        rejected = False
        try:
            check_ridx_index(repo_dir)
        except RuntimeError as e:
            rejected = True
            assert "RIDX index missing" in str(e)
            print(f"PASS: empty index directory rejected as expected ({e})")
        assert rejected, "Expected RuntimeError on missing index directory"

        # Test 2: Valid index with shard and manifest - expect success
        ridx_dir = repo_dir / "target" / "ridx"
        ridx_dir.mkdir(parents=True)
        manifest = {
            "sysroot_cas_key": "5368ac0ba3a3ce1e",
            "active_variant_id": "dev-test",
        }
        (ridx_dir / "ridx_manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
        (ridx_dir / "sample.ridx").write_bytes(b"MOCK_RIDX_SHARD")

        shards = count_index_shards(repo_dir)
        assert shards == 1, f"Expected 1 shard, got {shards}"

        checked = check_ridx_index(repo_dir)
        assert checked == 1, f"Expected checked shard count 1, got {checked}"
        print(f"PASS: valid index directory with shard accepted (shards={checked})")
        print("PASS: all index precondition checks passed")


if __name__ == "__main__":
    try:
        test_index_precondition()
    except AssertionError as exc:
        print(f"FAIL: {exc}", file=sys.stderr)
        sys.exit(1)
    sys.exit(0)
