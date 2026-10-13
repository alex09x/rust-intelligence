# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Unit test for driver provenance: the driver that built the index is resolved from the index
manifest, the ridx binary is never hashed in place of a missing driver, and the env override wins.
"""

import json
import os
import sys
import tempfile
from pathlib import Path

bench_dir = Path(__file__).resolve().parent
if str(bench_dir) not in sys.path:
    sys.path.insert(0, str(bench_dir))

import hashlib

from matrix_provenance import corpus_lockfile_sha256, index_driver_path, index_timestamp_epoch, resolve_driver
from provenance import fnv1a_64, get_driver_hash

DRIVER_BYTES = b"fake ridx-driver payload"
RIDX_BYTES = b"fake ridx payload"


def _write_manifest(repo_dir: Path, driver_path: Path, epoch: int) -> None:
    ridx_dir = repo_dir / "target" / "ridx"
    ridx_dir.mkdir(parents=True, exist_ok=True)
    manifest = {
        "sysroot_cas_key": "5368ac0ba3a3ce1e",
        "timestamp_epoch_secs": epoch,
        "dependency_shards": [
            {"name": "memchr", "compiler_args": [str(driver_path), "--crate-name", "memchr"]},
        ],
    }
    (ridx_dir / "ridx_manifest.json").write_text(json.dumps(manifest), encoding="utf-8")


def test_driver_resolved_from_index_manifest():
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        repo_dir = root / "corpus"
        bin_dir = root / "bench-clone" / "target" / "release"
        bin_dir.mkdir(parents=True)
        ridx_bin = bin_dir / "ridx"
        ridx_bin.write_bytes(RIDX_BYTES)
        workspace_driver = root / "workspace" / "target" / "release" / "ridx-driver"
        workspace_driver.parent.mkdir(parents=True)
        workspace_driver.write_bytes(DRIVER_BYTES)
        os.utime(workspace_driver, (1_791_776_746, 1_791_776_746))
        _write_manifest(repo_dir, workspace_driver, 1_791_791_746)

        os.environ.pop("RIDX_DRIVER_BIN", None)
        assert index_driver_path(repo_dir) == workspace_driver
        assert index_timestamp_epoch(repo_dir) == 1_791_791_746
        path, source = resolve_driver(repo_dir, ridx_bin)
        assert (path, source) == (workspace_driver, "index_manifest")
        assert get_driver_hash(ridx_bin, path) == f"{fnv1a_64(DRIVER_BYTES):016x}"
        assert path.stat().st_mtime <= 1_791_791_746
        print("PASS: driver resolved from the index manifest and hashed from that binary")


def test_missing_driver_is_unknown_not_ridx_hash():
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        repo_dir = root / "corpus"
        (repo_dir / "target" / "ridx").mkdir(parents=True)
        bin_dir = root / "bench-clone" / "target" / "release"
        bin_dir.mkdir(parents=True)
        ridx_bin = bin_dir / "ridx"
        ridx_bin.write_bytes(RIDX_BYTES)

        os.environ.pop("RIDX_DRIVER_BIN", None)
        assert index_driver_path(repo_dir) is None
        assert index_timestamp_epoch(repo_dir) is None
        assert resolve_driver(repo_dir, ridx_bin) == (None, "none")
        assert get_driver_hash(ridx_bin) is None
        assert get_driver_hash(ridx_bin, None) is None
        print("PASS: without a driver binary the hash is None, not the ridx fingerprint")


def test_sibling_driver_and_env_override():
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        repo_dir = root / "corpus"
        bin_dir = root / "target" / "release"
        bin_dir.mkdir(parents=True)
        ridx_bin = bin_dir / "ridx"
        ridx_bin.write_bytes(RIDX_BYTES)
        sibling = bin_dir / "ridx-driver"
        sibling.write_bytes(DRIVER_BYTES)

        os.environ.pop("RIDX_DRIVER_BIN", None)
        assert resolve_driver(repo_dir, ridx_bin) == (sibling, "sibling")
        assert get_driver_hash(ridx_bin) == f"{fnv1a_64(DRIVER_BYTES):016x}"

        env_driver = root / "env-driver"
        env_driver.write_bytes(b"env driver payload")
        os.environ["RIDX_DRIVER_BIN"] = str(env_driver)
        try:
            assert resolve_driver(repo_dir, ridx_bin) == (env_driver, "env")
            assert get_driver_hash(ridx_bin) == f"{fnv1a_64(b'env driver payload'):016x}"
        finally:
            os.environ.pop("RIDX_DRIVER_BIN", None)
        print("PASS: sibling driver is used when no manifest exists and RIDX_DRIVER_BIN overrides both")


def test_corpus_lockfile_sha256():
    with tempfile.TemporaryDirectory() as td:
        repo_dir = Path(td)
        assert corpus_lockfile_sha256(repo_dir) == "none"
        lock = repo_dir / "Cargo.lock"
        lock.write_bytes(b'[[package]]\nname = "trybuild"\nversion = "1.0.108"\n')
        assert corpus_lockfile_sha256(repo_dir) == hashlib.sha256(lock.read_bytes()).hexdigest()
        print("PASS: corpus lockfile sha256 is recorded, 'none' without a lockfile")


if __name__ == "__main__":
    test_driver_resolved_from_index_manifest()
    test_missing_driver_is_unknown_not_ridx_hash()
    test_sibling_driver_and_env_override()
    test_corpus_lockfile_sha256()
    print("PASS: all driver provenance tests passed")
