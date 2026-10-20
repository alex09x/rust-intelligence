# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Publication script for the matrix runs: sanitizes host-specific paths, writes the provenance
sidecar from the fields the runner recorded, and publishes the verified files into bench/published/.
"""

import argparse
import json
from pathlib import Path
from typing import Any, Dict, List

from publish_common import CANONICAL_REPOS, compute_sha256, sanitize_text, verify_clean_output

MATRIX_PROVENANCE_FIELDS = [
    "repo_commit", "corpus_lockfile_sha256", "ridx_commit", "ridx_binary_dirty", "driver_hash", "driver_build_commit",
    "driver_dirty", "driver_source", "driver_newer_than_index", "index_built_at",
    "rustc", "ra_version", "node", "taskset", "seed", "sample_count_per_method",
    "total_methods", "total_tests_per_server", "sysroot_cas_key", "sysroot_transplanted",
    "ridx_index_shards", "timestamp_start", "timestamp_end", "ridx_duration_sec", "ra_duration_sec",
    "ra_initial_wait_ms", "ra_canary_wait_ms", "ra_canary_attempts",
    "ra_quiescent_pre_query_wait_ms", "ra_canary_pre_query_wait_ms", "ra_canary_pre_query_attempts",
    "ridx_cancelled_on_timeout", "ra_cancelled_on_timeout",
]


def _write_text(path: Path, content: str) -> None:
    with open(path, "w", encoding="utf-8") as f:
        f.write(content)


def publish_matrix(src_dir: Path, pub_dir: Path, repo_key: str, repo_canon: str, dry_run: bool = False) -> bool:
    """Publishes matrix_<repo>_booster.json, its .positions.json and a provenance sidecar.

    The sidecar carries the provenance recorded by the matrix runner itself; nothing is
    inferred from file dates or the publishing host. Returns False when the raw JSON is absent.
    """
    src_file = src_dir / f"matrix_{repo_key}_booster.json"
    if not src_file.is_file():
        print(f"Skipping {repo_key} matrix: {src_file} not found")
        return False
    pos_src = src_dir / f"matrix_{repo_key}_booster.positions.json"
    dst_file = pub_dir / src_file.name
    pos_dst = pub_dir / pos_src.name
    sidecar_file = pub_dir / f"matrix_{repo_key}_booster.provenance.json"

    with open(src_file, "r", encoding="utf-8") as f:
        data = json.load(f)
    sanitization_rules: List[Dict[str, Any]] = []
    if data.get("repo") != repo_canon:
        sanitization_rules.append({"rule": f"repo -> {repo_canon}", "replacements": 1})
        data["repo"] = repo_canon
    dst_str, rules = sanitize_text(json.dumps(data, indent=2))
    sanitization_rules.extend(rules)
    verify_clean_output(dst_str, dst_file.name)

    pos_str = None
    pos_sha = None
    if pos_src.is_file():
        pos_str, pos_rules = sanitize_text(pos_src.read_text(encoding="utf-8"))
        sanitization_rules.extend({"rule": f"positions: {r['rule']}", "replacements": r["replacements"]} for r in pos_rules)
        verify_clean_output(pos_str, pos_dst.name)
        pos_sha = compute_sha256(pos_src)

    sidecar_data: Dict[str, Any] = {"dataset": f"matrix_{repo_key}_booster", "repo": repo_canon}
    for key in MATRIX_PROVENANCE_FIELDS:
        sidecar_data[key] = data.get(key, "n/a")
    sidecar_data["harness_commit"] = data.get("ridx_commit", "n/a")
    sidecar_data["harness_commit_note"] = "bench/ scripts and the ridx binary come from the same checkout commit"
    ts = str(data.get("timestamp_start", ""))
    sidecar_data["date"] = ts[:10] if len(ts) >= 10 else "n/a"
    sidecar_data["date_source"] = "timestamp_start recorded by the matrix runner"
    sidecar_data["source_sha256"] = compute_sha256(src_file)
    sidecar_data["positions_file"] = pos_dst.name if pos_str is not None else None
    sidecar_data["positions_source_sha256"] = pos_sha
    sidecar_data["sanitization"] = sanitization_rules
    sidecar_str = json.dumps(sidecar_data, indent=2)
    verify_clean_output(sidecar_str, sidecar_file.name)

    if dry_run:
        print(f"[dry-run] Would write {dst_file}, {sidecar_file}" + (f", {pos_dst}" if pos_str is not None else ""))
        return True
    _write_text(dst_file, dst_str)
    if pos_str is not None:
        _write_text(pos_dst, pos_str)
    _write_text(sidecar_file, sidecar_str)
    print(f"Published {repo_key} matrix to {dst_file} and {sidecar_file}")
    return True


def main() -> None:
    parser = argparse.ArgumentParser(description="Publish and sanitize benchmark results")
    parser.add_argument("--src", default="bench/results", help="Source directory containing raw results")
    parser.add_argument("--pub", default="bench/published", help="Destination publication directory")
    parser.add_argument("--dry-run", action="store_true", help="Perform sanity checks without writing files")
    args = parser.parse_args()

    src_dir = Path(args.src).resolve()
    pub_dir = Path(args.pub).resolve()
    if not args.dry_run:
        pub_dir.mkdir(parents=True, exist_ok=True)

    print(f"Publishing benchmark results from {src_dir} to {pub_dir}...")
    for repo_key in ("serde", "ripgrep", "tokio", "rust-analyzer", "bevy"):
        publish_matrix(src_dir, pub_dir, repo_key, CANONICAL_REPOS[repo_key], dry_run=args.dry_run)
    print("Done.")


if __name__ == "__main__":
    main()
