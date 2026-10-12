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

import json
import os
import subprocess
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict

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


def collect_matrix_provenance(
    repo_dir: Path,
    ridx_bin: Path,
    ra_bin: str,
    seed: int,
    timestamp_start: str,
    timestamp_end: str,
    repo_id: str = None,
) -> Dict[str, Any]:
    """Collects complete provenance metadata according to methodology specification."""
    cas_key = find_sysroot_cas_key(repo_dir)
    sysroot_trans = is_sysroot_transplanted(ridx_bin, cas_key) if cas_key else False

    ridx_commit, ridx_dirty = get_binary_build_info(ridx_bin)
    cand_driver = Path(ridx_bin).parent / "ridx-driver"
    driver_bin = os.environ.get("RIDX_DRIVER_BIN") or (cand_driver if cand_driver.is_file() else None)
    drv_commit, drv_dirty = get_binary_build_info(driver_bin) if driver_bin else (None, False)

    repo_root = Path(__file__).resolve().parent.parent
    ws_head = get_repo_commit(repo_root) or "unknown"

    return {
        "ridx_commit": ridx_commit or "unknown",
        "ridx_binary_dirty": ridx_dirty,
        "driver_hash": get_driver_hash(ridx_bin) or "unknown",
        "driver_build_commit": drv_commit or "unknown",
        "driver_dirty": drv_dirty,
        "workspace_head": ws_head,
        "workspace_head_note": "Reference only: git HEAD of workspace running the benchmark",
        "rustc": get_rustc_version(),
        "ra_version": get_ra_version(ra_bin),
        "node": get_node_info(),
        "taskset": get_taskset_affinity(),
        "seed": seed,
        "repo": normalize_repo_identifier(repo_dir, repo_id),
        "repo_commit": get_repo_commit(repo_dir) or "unknown",
        "sysroot_cas_key": cas_key or "none",
        "sysroot_transplanted": sysroot_trans,
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
