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


def _get_driver_env():
    env = dict(os.environ)
    if "LD_LIBRARY_PATH" not in env or "DYLD_LIBRARY_PATH" not in env:
        try:
            import subprocess
            out = subprocess.run(["rustc", "--print", "sysroot"], capture_output=True, text=True, check=False)
            if out.returncode == 0 and out.stdout.strip():
                lib_p = Path(out.stdout.strip()) / "lib"
                if lib_p.is_dir():
                    env.setdefault("LD_LIBRARY_PATH", str(lib_p))
                    env.setdefault("DYLD_LIBRARY_PATH", str(lib_p))
        except Exception:
            pass
    return env


def is_driver_transplanted(cas_key: str, curr_bin: str = None) -> bool:
    """Verifies that the driver binary can execute and matches the expected sysroot CAS key."""
    if not cas_key:
        return True
    driver_bin = os.environ.get("RUSTC_WORKSPACE_WRAPPER") or os.environ.get("RIDX_DRIVER_BIN")
    if not driver_bin and curr_bin:
        curr_path = Path(curr_bin)
        cand_driver = curr_path.parent / "ridx-driver"
        if cand_driver.is_file():
            driver_bin = str(cand_driver)
        elif curr_path.is_file():
            driver_bin = str(curr_path)
    if not driver_bin or not Path(driver_bin).is_file():
        return True
    try:
        env = _get_driver_env()
        import subprocess
        res = subprocess.run([driver_bin, "--sysroot-cas-key"], capture_output=True, text=True, env=env, check=False)
        if res.returncode == 0 and res.stdout.strip():
            return res.stdout.strip() != cas_key
        return True
    except Exception:
        return True


def is_sysroot_transplanted(curr_bin, cas_key):
    """Returns False if cas_key matches the active driver's computed sysroot CAS key, True otherwise."""
    return is_driver_transplanted(cas_key, curr_bin)


def get_binary_build_info(bin_path):
    """Returns (commit, is_dirty) from binary via --ridx-version / --version, or (None, False)."""
    if not bin_path:
        return None, False
    try:
        import subprocess
        import re
        env = _get_driver_env()
        for flag in ["--ridx-version", "--version"]:
            try:
                res = subprocess.run([str(bin_path), flag], capture_output=True, text=True, env=env, check=False)
                if res.returncode == 0 and res.stdout.strip():
                    m = re.search(r"commit:\s*([0-9a-fA-F]{40}|unknown),\s*dirty:\s*(true|false)", res.stdout)
                    if m:
                        c = m.group(1)
                        d = m.group(2).lower() == "true"
                        return (c if c != "unknown" else None), d
            except Exception:
                continue
    except Exception:
        pass
    return None, False


def get_driver_build_info(curr_bin=None):
    """Returns (commit, is_dirty) for the driver and ridx binaries."""
    driver_bin = os.environ.get("RIDX_DRIVER_BIN") or os.environ.get("RUSTC_WORKSPACE_WRAPPER")
    if not driver_bin and curr_bin:
        cand_driver = Path(curr_bin).parent / "ridx-driver"
        if cand_driver.is_file():
            driver_bin = str(cand_driver)

    drv_commit, drv_dirty = get_binary_build_info(driver_bin) if driver_bin else (None, False)
    curr_commit, curr_dirty = get_binary_build_info(curr_bin) if curr_bin else (None, False)

    if driver_bin and curr_bin:
        if not drv_commit or not curr_commit or drv_commit != curr_commit:
            commit = None
        else:
            commit = drv_commit
    elif driver_bin:
        commit = drv_commit
    else:
        commit = curr_commit

    dirty = drv_dirty or curr_dirty
    return commit, dirty


def get_driver_build_commit(curr_bin=None):
    """Returns 40-char git commit of the workspace that built the driver/ridx, or None."""
    commit, _ = get_driver_build_info(curr_bin)
    return commit


def get_node_info():
    """Detects host node hostname, CPU model, core/thread count, and RAM."""
    if "BENCH_NODE" in os.environ:
        return os.environ["BENCH_NODE"]
    import socket
    hostname = socket.gethostname()
    cpu_model = "unknown"
    threads = os.cpu_count() or 0
    try:
        with open("/proc/cpuinfo", "r", encoding="utf-8", errors="replace") as f:
            for line in f:
                if "model name" in line:
                    cpu_model = line.split(":", 1)[1].strip()
                    break
    except Exception:
        import platform
        cpu_model = platform.processor() or "unknown"

    ram_str = "unknown"
    try:
        with open("/proc/meminfo", "r", encoding="utf-8", errors="replace") as f:
            for line in f:
                if line.startswith("MemTotal:"):
                    kb = int(line.split(":", 1)[1].strip().split()[0])
                    ram_str = f"{round(kb / (1024 * 1024))} GB RAM"
                    break
    except Exception:
        pass

    parts = [cpu_model]
    if threads:
        parts.append(f"{threads} threads")
    if ram_str != "unknown":
        parts.append(ram_str)
    return f"{hostname} ({', '.join(parts)})"


def get_taskset_affinity():
    """Detects CPU affinity / taskset for the current process."""
    if "BENCH_TASKSET" in os.environ:
        return os.environ["BENCH_TASKSET"]
    try:
        if hasattr(os, "sched_getaffinity"):
            aff = sorted(os.sched_getaffinity(0))
            if aff:
                if len(aff) > 1 and aff == list(range(aff[0], aff[-1] + 1)):
                    return f"{aff[0]}-{aff[-1]}"
                return ",".join(map(str, aff))
    except Exception:
        pass
    try:
        with open("/proc/self/status", "r", encoding="utf-8", errors="replace") as f:
            for line in f:
                if line.startswith("Cpus_allowed_list:"):
                    return line.split(":", 1)[1].strip()
    except Exception:
        pass
    return "unknown"


def get_node_source():
    """Returns 'env' if BENCH_NODE or BENCH_TASKSET is explicitly set in environment, else 'detected'."""
    if "BENCH_NODE" in os.environ or "BENCH_TASKSET" in os.environ:
        return "env"
    return "detected"



