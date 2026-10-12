# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Publication script for benchmark results.
Sanitizes host-specific paths, structures provenance sidecars, and publishes
verified benchmark artifacts into bench/published/.
"""

import argparse
import json
import re
import subprocess
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict

CANONICAL_REPOS = {
    "serde": "serde-rs/serde",
    "ripgrep": "BurntSushi/ripgrep",
    "tokio": "tokio-rs/tokio",
    "bevy": "bevyengine/bevy",
    "rust-analyzer": "rust-lang/rust-analyzer",
}

ROADMAP_DIMENSION_MAP = {
    "initial_indexing": [
        "cold_cargo_check_sec",
        "cargo_check_peak_rss_mb",
        "cargo_check_peak_fds",
        "cargo_check_peak_threads",
        "cold_ridx_index_sec",
        "ridx_index_peak_rss_mb",
        "ridx_index_peak_fds",
        "ridx_index_peak_threads",
        "ridx_index_size_bytes",
        "ra_scip_build_sec",
        "ra_scip_peak_rss_mb",
        "ra_scip_peak_fds",
        "ra_scip_peak_threads",
        "ra_scip_size_bytes",
    ],
    "warm_no_op": [
        "warm_ridx_index_sec",
    ],
    "incremental_change": [
        "w1_commit_walk",
        "w2_large_jumps",
    ],
    "open_and_first_query": [
        "ridx_init_ms",
        "ridx_warmup_ms",
        "ra_init_ms",
        "ra_warmup_ms",
        "scip_load_ms",
    ],
    "warm_requests": [
        "metrics",
        "readonly_total",
        "readonly_matched_strict",
        "readonly_match_rate_strict_pct",
        "readonly_matched_line_level",
        "readonly_match_rate_line_level_pct",
        "matched_scenarios_strict",
        "match_rate_strict_pct",
        "matched_scenarios_line_level",
        "match_rate_line_level_pct",
        "matched_scenarios",
        "total_scenarios",
        "match_rate_pct",
        "scip_matched_scenarios",
        "scip_total_scenarios",
        "scip_match_rate_pct",
    ],
    "storage_and_memory": [
        "ridx_peak_rss_mb",
        "ridx_peak_rss_anon_mb",
        "ridx_peak_rss_file_mb",
        "ridx_peak_open_fds",
        "ridx_peak_threads",
        "ridx_peak_ridx_open_fds",
        "ridx_peak_ridx_mapped_regions",
        "ra_peak_rss_mb",
        "ra_peak_rss_anon_mb",
        "ra_peak_rss_file_mb",
        "ra_peak_open_fds",
        "ra_peak_threads",
        "scip_peak_rss_mb",
        "scip_peak_open_fds",
        "scip_peak_threads",
        "w3_multi_worktree",
    ],
}

FORBIDDEN_PATTERNS = ["/home/", "/tmp/", "/Users/", "alex09x"]


def sanitize_text(text: str) -> str:
    """Sanitizes paths according to canonical mapping and privacy requirements."""
    for repo_short, canonical in CANONICAL_REPOS.items():
        text = text.replace(f"/tmp/mitts-bench/{repo_short}/", f"{canonical}/")
    text = re.sub(r"/tmp/mitts-bench/([^/]+)/", r"\1/", text)
    text = re.sub(r"/home/[^/]+/\.cargo/registry/", "<cargo-registry>/", text)
    text = re.sub(
        r"/home/[^/]+/\.rustup/toolchains/[^/]+/lib/rustlib/",
        "<rustlib>/",
        text,
    )
    text = re.sub(r"/home/[^/]+/", "<user-home>/", text)
    text = re.sub(r"/tmp/", "<tmp>/", text)
    text = re.sub(r"alex09x", "<user>", text)
    return text


def verify_clean_output(content: str, filename: str) -> None:
    """Verifies that no host-private patterns appear in published text."""
    for pattern in FORBIDDEN_PATTERNS:
        if pattern in content:
            count = content.count(pattern)
            raise RuntimeError(
                f"Privacy check failed for {filename}: found {count} occurrences of '{pattern}'"
            )


def get_commit_date(commit_hash: str) -> str:
    """Queries ISO-8601 commit date from git log."""
    try:
        res = subprocess.run(
            ["git", "show", "-s", "--format=%cI", commit_hash],
            capture_output=True,
            text=True,
            check=False,
        )
        if res.returncode == 0 and res.stdout.strip():
            return res.stdout.strip()
    except Exception:
        pass
    return "unknown"


def publish_pilot_results(src_dir: Path, pub_dir: Path, dry_run: bool = False) -> None:
    """Publishes pilot benchmark results and generates provenance sidecar."""
    src_file = src_dir / "pilot_results.jsonl"
    if not src_file.is_file():
        print(f"Skipping pilot: {src_file} not found")
        return

    dst_file = pub_dir / "pilot_5repos_2026-10-08.jsonl"
    sidecar_file = pub_dir / "pilot_5repos_2026-10-08.provenance.json"

    repo_commits = {}
    driver_hashes = {}
    ra_version = "unknown"
    timestamps = []

    lines = []
    first_record = None
    with open(src_file, "r", encoding="utf-8") as f:
        for line in f:
            if not line.strip():
                continue
            lines.append(line)
            record = json.loads(line)
            if first_record is None:
                first_record = record
            repo = record.get("repo")
            if repo:
                repo_commits[repo] = record.get("commit", "unknown")
                driver_hashes[repo] = record.get("driver_sha256", "unknown")
                if record.get("ra_version"):
                    ra_version = record.get("ra_version")
            if "timestamp" in record:
                timestamps.append(record["timestamp"])

    if first_record is None:
        raise RuntimeError(f"Empty pilot results file: {src_file}")

    # Build and validate ROADMAP measurements map from actual data
    measurements: Dict[str, list] = {}
    for dim, keys in ROADMAP_DIMENSION_MAP.items():
        dim_keys = []
        for k in keys:
            if k not in first_record:
                raise RuntimeError(
                    f"Declared ROADMAP key '{k}' missing from pilot results first record"
                )
            dim_keys.append(k)
        measurements[dim] = dim_keys

    # Compute chronology
    if timestamps:
        min_ts = min(timestamps)
        max_ts = max(timestamps)
        collected_from = datetime.fromtimestamp(min_ts, tz=timezone.utc).isoformat()
        collected_to = datetime.fromtimestamp(max_ts, tz=timezone.utc).isoformat()
    else:
        collected_from = "unknown"
        collected_to = "unknown"

    # Build driver_note dynamically from driver_sha256 dictionary
    hash_to_repos: Dict[str, list] = {}
    for r, h in driver_hashes.items():
        hash_to_repos.setdefault(h, []).append(r)
    driver_note_parts = []
    for h, rlist in sorted(hash_to_repos.items(), key=lambda x: len(x[1])):
        driver_note_parts.append(f"{', '.join(rlist)} collected using driver sha256 {h}")
    driver_note = "; ".join(driver_note_parts)

    date_53e8364 = get_commit_date("53e8364")
    date_844eac0 = get_commit_date("844eac0")
    ridx_commit_note = (
        f"Pilot benchmark collected between {collected_from} and {collected_to}, "
        f"prior to commits 844eac0 ({date_844eac0}) and 53e8364 ({date_53e8364}). "
        "Exact RIDX git commit not recorded in telemetry."
    )

    sidecar_data: Dict[str, Any] = {
        "dataset": "pilot_5repos_2026-10-08",
        "date": "2026-10-08",
        "collected_from": collected_from,
        "collected_to": collected_to,
        "ridx_commit": "unknown",
        "ridx_commit_note": ridx_commit_note,
        "driver_sha256": driver_hashes,
        "driver_note": driver_note,
        "rustc": "unknown",
        "ra_version": ra_version,
        "node": "unknown",
        "taskset": "unknown",
        "repo_commits": repo_commits,
        "measurements": measurements,
    }

    clean_lines = sanitize_text("".join(lines))
    verify_clean_output(clean_lines, dst_file.name)
    sidecar_str = json.dumps(sidecar_data, indent=2)
    verify_clean_output(sidecar_str, sidecar_file.name)

    if dry_run:
        print(f"[dry-run] Would write {len(lines)} lines to {dst_file}")
        print(f"[dry-run] Would write provenance sidecar to {sidecar_file}")
        return

    with open(dst_file, "w", encoding="utf-8") as f:
        f.write(clean_lines)
    with open(sidecar_file, "w", encoding="utf-8") as f:
        f.write(sidecar_str)
    print(f"Published pilot results to {dst_file} and {sidecar_file}")


def publish_serde_matrix(src_dir: Path, pub_dir: Path, dry_run: bool = False) -> None:
    """Sanitizes serde booster matrix JSON and produces provenance sidecar."""
    src_file = src_dir / "matrix_serde_booster.json"
    if not src_file.is_file():
        print(f"Skipping serde matrix: {src_file} not found")
        return

    dst_file = pub_dir / "matrix_serde_booster.json"
    sidecar_file = pub_dir / "matrix_serde_booster.provenance.json"

    with open(src_file, "r", encoding="utf-8") as f:
        data = json.load(f)

    data["repo"] = "serde-rs/serde"

    sidecar_data = {
        "dataset": "matrix_serde_booster",
        "repo": "serde-rs/serde",
        "repo_commit": "unknown",
        "ridx_commit": "unknown",
        "ridx_commit_note": (
            "Matrix run dated 2026-10-08 12:47 PDT on booster; exact RIDX build commit "
            "not recorded in raw JSON."
        ),
        "driver_hash": "unknown",
        "rustc": "unknown",
        "ra_version": "unknown",
        "node": "booster",
        "node_source": "file name",
        "date": "2026-10-08",
        "date_source": "mtime of raw file on publishing host",
        "taskset": "unknown",
        "seed": "unknown",
        "duration_sec": {
            "ridx": data.get("ridx_duration_sec"),
            "rust_analyzer": data.get("ra_duration_sec"),
        },
    }

    dst_str = json.dumps(data, indent=2)
    verify_clean_output(dst_str, dst_file.name)
    sidecar_str = json.dumps(sidecar_data, indent=2)
    verify_clean_output(sidecar_str, sidecar_file.name)

    if dry_run:
        print(f"[dry-run] Would write sanitized serde matrix to {dst_file}")
        print(f"[dry-run] Would write provenance sidecar to {sidecar_file}")
        return

    with open(dst_file, "w", encoding="utf-8") as f:
        f.write(dst_str)
    with open(sidecar_file, "w", encoding="utf-8") as f:
        f.write(sidecar_str)
    print(f"Published serde matrix to {dst_file} and {sidecar_file}")


def publish_discrepancies(src_dir: Path, pub_dir: Path, dry_run: bool = False) -> None:
    """Sanitizes discrepancies.jsonl by stripping private host paths."""
    src_file = src_dir / "discrepancies.jsonl"
    if not src_file.is_file():
        print(f"Skipping discrepancies: {src_file} not found")
        return

    dst_file = pub_dir / "discrepancies.jsonl"

    clean_output = sanitize_text(src_file.read_text(encoding="utf-8"))
    verify_clean_output(clean_output, dst_file.name)

    if dry_run:
        line_count = len(clean_output.splitlines())
        print(f"[dry-run] Would write {line_count} sanitized lines to {dst_file}")
        return

    with open(dst_file, "w", encoding="utf-8") as f:
        f.write(clean_output)
    print(f"Published discrepancies to {dst_file}")


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
    publish_pilot_results(src_dir, pub_dir, dry_run=args.dry_run)
    publish_serde_matrix(src_dir, pub_dir, dry_run=args.dry_run)
    publish_discrepancies(src_dir, pub_dir, dry_run=args.dry_run)
    print("Done.")


if __name__ == "__main__":
    main()
