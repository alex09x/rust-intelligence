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


CANONICAL_REPO_COMMITS = {
    "serde": "69072f24232f431911ba2c41da69a1da10e805a1",
    "bevy": "68c80f3b19ddf9e4ae1ce7dd8b5a68273204c2e8",
}

ROOT_CAUSES = {
    "comp_colon_41_RangeVisitor": "Shadowing: sysroot core::ops::range resolved before local mod range at serde_core/src/de/impls.rs:2463 (5 sysroot items vs 2 local).",
    "comp_colon_37_SocketAddr": "Re-export omission: std::net re-exports core::net::SocketAddr, but std.ridx excluded core re-exports (1 item vs 13).",
}

EXCLUDED_SCENARIOS = {
    "bevyengine/bevy": {
        "comp_colon_10_Greater": "Sampling position on clean 68c80f3b falls on younger_before_ord_wrap (multi-line assert_eq) instead of Ordering::Greater.",
    }
}

REPO_SLUG_TO_NAME = {"serde-rs/serde": "serde", "bevyengine/bevy": "bevy"}


def load_baseline(baseline_file):
    with open(baseline_file, "r", encoding="utf-8") as f:
        return {d["repo"]: d for d in (json.loads(l) for l in f if l.strip())}



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
    return Path.home() / "prod-code-storage" / "bench_repos" / name


def get_repo_commit(rpath):
    """Returns 40-char git commit hash of the benchmark repository."""
    try:
        import subprocess
        res = subprocess.run(
            ["git", "-C", str(rpath), "rev-parse", "HEAD"],
            capture_output=True,
            text=True,
            check=False,
        )
        if res.returncode == 0 and res.stdout.strip():
            return res.stdout.strip()
    except Exception:
        pass
    return None


def check_repo_clean(rpath):
    """Checks whether the git repository has no uncommitted changes."""
    try:
        import subprocess
        res = subprocess.run(
            ["git", "-C", str(rpath), "status", "--porcelain", "--untracked-files=no"],
            capture_output=True,
            text=True,
            check=False,
        )
        return res.returncode == 0 and not res.stdout.strip()
    except Exception:
        return False


def verify_repo_commit(name, rpath):
    """Returns (commit, is_valid_canonical_checkout)."""
    commit = get_repo_commit(rpath)
    expected = CANONICAL_REPO_COMMITS.get(name)
    if not commit or not expected or commit.lower() != expected.lower():
        return commit, False
    if not check_repo_clean(rpath):
        return commit, False
    return commit, True


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


def is_sysroot_transplanted(curr_bin, cas_key):
    """Returns False if cas_key matches the active driver's computed sysroot CAS key, True otherwise."""
    if not cas_key:
        return True
    driver_bin = os.environ.get("RIDX_DRIVER_BIN")
    if not driver_bin or not Path(driver_bin).is_file():
        curr_path = Path(curr_bin)
        cand_driver = curr_path.parent / "ridx-driver"
        if cand_driver.is_file():
            driver_bin = str(cand_driver)
        elif curr_path.is_file():
            driver_bin = str(curr_path)
    if not driver_bin or not Path(driver_bin).is_file():
        return True
    try:
        env = dict(os.environ)
        if "LD_LIBRARY_PATH" not in env:
            import subprocess
            out = subprocess.run(["rustc", "--print", "sysroot"], capture_output=True, text=True, check=False)
            if out.returncode == 0 and out.stdout.strip():
                lib_p = Path(out.stdout.strip()) / "lib"
                if lib_p.is_dir():
                    env["LD_LIBRARY_PATH"] = str(lib_p)
        import subprocess
        res = subprocess.run([driver_bin, "--sysroot-cas-key"], capture_output=True, text=True, env=env, check=False)
        if res.returncode == 0 and res.stdout.strip():
            return res.stdout.strip() != cas_key
        return True
    except Exception:
        return True


def get_driver_build_commit(curr_bin=None):
    """Returns 40-char git commit of the workspace that built the driver/ridx, or None."""
    try:
        import subprocess
        ws_root = Path(__file__).resolve().parent.parent
        res = subprocess.run(
            ["git", "-C", str(ws_root), "rev-parse", "HEAD"],
            capture_output=True,
            text=True,
            check=False,
        )
        if res.returncode == 0 and res.stdout.strip():
            return res.stdout.strip()
    except Exception:
        pass
    return None

