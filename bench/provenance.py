# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Provenance metadata inspection for RIDX and sysroot cache artifacts."""

import hashlib
import json
import os
from datetime import datetime, timezone
from pathlib import Path


def find_default_repo(name, bench_dir=None):
    """Discovers local or booster benchmark repository paths by precedence."""
    env_key = f"BENCH_{name.upper()}_DIR"
    if env_key in os.environ:
        return Path(os.environ[env_key])
    candidates = [
        Path.home() / "prod-code-storage" / "bench_repos" / name,
        Path("/tmp") / name,
    ]
    if bench_dir:
        candidates.insert(1, Path(bench_dir).parent.parent / "bench_repos" / name)
    for c in candidates:
        if c.is_dir():
            return c
    return Path("/tmp") / name


def find_sysroot_cas_key(rpath):
    """Finds the active sysroot CAS key from index manifests in target/ridx."""
    root = Path(rpath) / "target" / "ridx"
    vdir = root / "variants"
    cands = [root / "ridx_manifest.json"] + (
        sorted(vdir.glob("*/variant_manifest.json"), reverse=True) if vdir.is_dir() else []
    )
    for mf in cands:
        if mf.is_file():
            try:
                k = json.loads(mf.read_text(encoding="utf-8")).get("sysroot_cas_key")
                if k:
                    return k
            except Exception:
                pass
    return None


def fnv1a_64(data: bytes) -> int:
    """Computes standard FNV-1a 64-bit hash matching ridx-format."""
    h = 0xCBF29CE484222325
    for b in data:
        h = ((h ^ b) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h


def get_driver_hash(curr_bin):
    """Returns 64-bit hex fingerprint of the active driver binary."""
    driver_bin = os.environ.get("RIDX_DRIVER_BIN")
    if driver_bin and Path(driver_bin).is_file():
        p = Path(driver_bin)
    else:
        curr_path = Path(curr_bin)
        cand_driver = curr_path.parent / "ridx-driver"
        if cand_driver.is_file():
            p = cand_driver
        elif curr_path.is_file():
            p = curr_path
        else:
            return None
    try:
        b = p.read_bytes()
        return f"{fnv1a_64(b):016x}"
    except Exception:
        return None


def get_sysroot_std_metadata(cas_key):
    """Returns ISO UTC mtime and sha256 prefix for cached std.ridx shard."""
    if not cas_key:
        return None, None
    cache_base = os.environ.get("RIDX_CACHE_DIR")
    if cache_base:
        sysroot_dir = Path(cache_base) / "sysroot" / cas_key
    else:
        sysroot_dir = Path.home() / ".cache" / "ridx" / "v3" / "sysroot" / cas_key
    std_ridx = sysroot_dir / "std.ridx"
    if not std_ridx.is_file():
        return None, None
    try:
        mtime = datetime.fromtimestamp(std_ridx.stat().st_mtime, tz=timezone.utc).isoformat()
        sha = hashlib.sha256(std_ridx.read_bytes()).hexdigest()[:16]
        return mtime, sha
    except Exception:
        return None, None
