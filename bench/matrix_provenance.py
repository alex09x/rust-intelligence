# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Provenance collection and scenario position recording for LSP matrix benchmarks.
"""

import hashlib
import json
import os
import subprocess
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, Optional, Tuple

from provenance import (
    _get_driver_env,
    find_sysroot_cas_key,
    get_binary_build_info,
    get_driver_hash,
    get_node_info,
    get_repo_commit,
    get_taskset_affinity,
    is_sysroot_transplanted,
)

CANONICAL_REPOS = {
    "serde": "serde-rs/serde",
    "ripgrep": "BurntSushi/ripgrep",
    "tokio": "tokio-rs/tokio",
    "bevy": "bevyengine/bevy",
    "rust-analyzer": "rust-lang/rust-analyzer",
}


def get_rustc_version() -> str:
    """Returns rustc version string or 'unknown' using driver environment."""
    try:
        env = _get_driver_env()
        res = subprocess.run(["rustc", "--version"], capture_output=True, text=True, env=env, check=False)
        if res.returncode == 0 and res.stdout.strip():
            return res.stdout.strip()
    except Exception:
        pass
    return "unknown"


def get_ra_version(ra_bin: str) -> str:
    """Returns rust-analyzer version string or 'unknown'."""
    try:
        res = subprocess.run([str(ra_bin), "--version"], capture_output=True, text=True, check=False)
        if res.returncode == 0 and res.stdout.strip():
            return res.stdout.strip()
    except Exception:
        pass
    return "unknown"


def normalize_repo_identifier(repo_path: Path, repo_id: str = None) -> str:
    """Returns canonical owner/name for known benchmark repositories or relative name."""
    if repo_id:
        return repo_id
    repo_name = repo_path.name
    if repo_name in CANONICAL_REPOS:
        return CANONICAL_REPOS[repo_name]
    return repo_name


def count_index_shards(repo_dir: Path) -> int:
    """Counts *.ridx shard files in target/ridx plus linked external dependency shards in ridx_manifest.json."""
    root = Path(repo_dir) / "target" / "ridx"
    if not root.is_dir():
        return 0
    ws_shards = sum(1 for _ in root.rglob("*.ridx"))
    manifest = _read_index_manifest(repo_dir)
    dep_shards = len(manifest.get("dependency_shards") or []) if manifest else 0
    return ws_shards + dep_shards


def check_ridx_index(repo_dir: Path) -> int:
    """Verifies that the RIDX index and sysroot CAS key exist, returning shard count."""
    shards = count_index_shards(repo_dir)
    cas_key = find_sysroot_cas_key(repo_dir)
    if shards == 0 or not cas_key:
        raise RuntimeError(
            f"RIDX index missing for {repo_dir}: found {shards} shards, sysroot_cas_key={cas_key or 'none'}. "
            "Index must be built before running matrix benchmark."
        )
    return shards


def _read_index_manifest(repo_dir: Path) -> Optional[Dict[str, Any]]:
    mf = Path(repo_dir) / "target" / "ridx" / "ridx_manifest.json"
    if not mf.is_file():
        return None
    try:
        data = json.loads(mf.read_text(encoding="utf-8"))
    except Exception:
        return None
    return data if isinstance(data, dict) else None


def index_driver_path(repo_dir: Path) -> Optional[Path]:
    """Returns the ridx-driver binary recorded in the index manifest's compiler_args, if any."""
    data = _read_index_manifest(repo_dir)
    if data is None:
        return None
    for shard in data.get("dependency_shards") or []:
        args = shard.get("compiler_args") or []
        if args and Path(str(args[0])).name.startswith("ridx-driver"):
            return Path(str(args[0]))
    return None


def index_timestamp_epoch(repo_dir: Path) -> Optional[int]:
    """Returns the index manifest's timestamp_epoch_secs, if recorded."""
    data = _read_index_manifest(repo_dir)
    if data is None:
        return None
    ts = data.get("timestamp_epoch_secs")
    return int(ts) if isinstance(ts, (int, float)) else None


def corpus_lockfile_sha256(repo_dir: Path) -> str:
    """Returns the sha256 of the corpus Cargo.lock, which fixes the dependency versions the index
    was built against, or "none" when the corpus has no lockfile."""
    lock = Path(repo_dir) / "Cargo.lock"
    if not lock.is_file():
        return "none"
    return hashlib.sha256(lock.read_bytes()).hexdigest()


def resolve_driver(repo_dir: Path, ridx_bin: Path) -> Tuple[Optional[Path], str]:
    """Locates the driver that built the index: RIDX_DRIVER_BIN, then the manifest, then a sibling.

    Returns (path, source) with source in {"env", "index_manifest", "sibling", "none"}.
    """
    env_driver = os.environ.get("RIDX_DRIVER_BIN")
    if env_driver and Path(env_driver).is_file():
        return Path(env_driver), "env"
    recorded = index_driver_path(repo_dir)
    if recorded is not None and recorded.is_file():
        return recorded, "index_manifest"
    sibling = Path(ridx_bin).parent / "ridx-driver"
    if sibling.is_file():
        return sibling, "sibling"
    return None, "none"


def collect_matrix_provenance(
    repo_dir: Path,
    ridx_bin: Path,
    ra_bin: str,
    seed: int,
    timestamp_start: str,
    timestamp_end: str,
    repo_id: str = None,
) -> Dict[str, Any]:
    """Collects complete provenance metadata according to methodology specification.

    The driver fields describe the ridx-driver that built the index being queried, resolved by
    `resolve_driver`; they are "unknown" when no driver binary can be located.
    """
    cas_key = find_sysroot_cas_key(repo_dir)
    driver_bin, driver_source = resolve_driver(repo_dir, ridx_bin)
    sysroot_trans = is_sysroot_transplanted(str(driver_bin), cas_key) if (cas_key and driver_bin) else cas_key is None

    ridx_commit, ridx_dirty = get_binary_build_info(ridx_bin)
    drv_commit, drv_dirty = get_binary_build_info(driver_bin) if driver_bin else (None, False)
    index_epoch = index_timestamp_epoch(repo_dir)
    index_built_at = (
        datetime.fromtimestamp(index_epoch, tz=timezone.utc).isoformat() if index_epoch is not None else "unknown"
    )
    driver_newer_than_index = (
        driver_bin is not None and index_epoch is not None and driver_bin.stat().st_mtime > index_epoch
    )

    repo_root = Path(__file__).resolve().parent.parent
    ws_head = get_repo_commit(repo_root) or "unknown"

    return {
        "ridx_commit": ridx_commit or "unknown",
        "ridx_binary_dirty": ridx_dirty,
        "driver_hash": get_driver_hash(ridx_bin, driver_bin) or "unknown",
        "driver_build_commit": drv_commit or "unknown",
        "driver_dirty": drv_dirty,
        "driver_source": driver_source,
        "driver_newer_than_index": driver_newer_than_index,
        "index_built_at": index_built_at,
        "workspace_head": ws_head,
        "workspace_head_note": "Reference only: git HEAD of workspace running the benchmark",
        "rustc": get_rustc_version(),
        "ra_version": get_ra_version(ra_bin),
        "node": get_node_info(),
        "taskset": get_taskset_affinity(),
        "seed": seed,
        "repo": normalize_repo_identifier(repo_dir, repo_id),
        "repo_commit": get_repo_commit(repo_dir) or "unknown",
        "corpus_lockfile_sha256": corpus_lockfile_sha256(repo_dir),
        "sysroot_cas_key": cas_key or "none",
        "sysroot_transplanted": sysroot_trans,
        "ridx_index_shards": count_index_shards(repo_dir),
        "timestamp_start": timestamp_start,
        "timestamp_end": timestamp_end,
    }


def save_scenarios_positions(
    scenarios: Dict[str, list],
    out_path: Path,
    repo_dir: Path,
    seed: int = None,
    sample_count: int = None,
) -> None:
    """Saves selected benchmark positions (file, line, character, symbol) next to results."""
    records = {}
    total_samples = 0
    for method_key, items in scenarios.items():
        method_records = []
        for it in items:
            total_samples += 1
            loc = it.get("location", [None, 0, 0])
            raw_file = loc[0] or it.get("file", "")
            # Make file path relative to repo_dir for privacy and portability
            try:
                rel_file = str(Path(raw_file).relative_to(repo_dir))
            except Exception:
                rel_file = Path(raw_file).name if raw_file else ""

            method_records.append({
                "name": it.get("name", ""),
                "file": rel_file,
                "line": loc[1] if len(loc) > 1 else 0,
                "character": loc[2] if len(loc) > 2 else 0,
                "symbol": it.get("target"),
            })
        records[method_key] = method_records

    payload = {
        "seed": seed,
        "sample_count": sample_count if sample_count is not None else total_samples,
        "positions": records,
    }

    out_path.parent.mkdir(parents=True, exist_ok=True)
    with open(out_path, "w", encoding="utf-8") as f:
        json.dump(payload, f, indent=2)
