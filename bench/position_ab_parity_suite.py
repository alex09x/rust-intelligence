#!/usr/bin/env python3
# ridx — Rust Indexer & Code Intelligence
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0

"""Run and aggregate the 38-method x 7-corpus LSP parity suite (`position_ab_parity.py`)."""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import os
import shlex
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any

CORPORA = ["serde", "ripgrep", "tokio", "rust-analyzer", "bevy", "clap", "regex"]

METHOD_GROUPS: dict[int, str] = {
    1: (
        "definition,declaration,type_definition,implementation,references,"
        "document_highlight,hover,signature_help,inlay_hint,completion,"
        "code_lens,selection_range"
    ),
    2: "document_symbol,workspace_symbol,folding_range,semantic_tokens_full",
    3: (
        "document_link,semantic_tokens_delta,semantic_tokens_range,"
        "prepare_call_hierarchy,incoming_calls,outgoing_calls,"
        "prepare_type_hierarchy,supertypes,subtypes,completion_resolve,"
        "prepare_rename"
    ),
    4: (
        "rename,code_action,code_action_resolve,formatting,range_formatting,"
        "on_type_formatting,document_diagnostic,workspace_diagnostic,"
        "code_lens_resolve,linked_editing_range,moniker"
    ),
}


def ra_cache_subdir(group_id: int) -> str:
    if group_id in (1, 2):
        return "ra_" + METHOD_GROUPS[group_id].replace(",", "_")
    return f"ra_rest{group_id - 2}"


def resolve_positions_file(review_dir: Path, corpus: str) -> Path:
    candidates = [
        review_dir.parent / f"matrix_{corpus}_booster.positions.json",
        review_dir / f"matrix_{corpus}_booster.positions.json",
        review_dir / "b83" / "published" / f"matrix_{corpus}_booster.positions.json",
    ]
    for cand in candidates:
        if cand.is_file():
            return cand
    raise FileNotFoundError(f"positions file not found for {corpus}: {candidates[0]}")


def run_unshare_cmd(
    repo_dir: Path,
    target_dir: Path,
    cache_dir: Path,
    inner_cmd: str,
    serde_lockfile: Path | None,
    log_path: Path,
    ridx_bin: Path | None = None,
    timeout_s: int = 900,
) -> int:
    target_dir.mkdir(parents=True, exist_ok=True)
    cache_dir.mkdir(parents=True, exist_ok=True)
    binds = []
    if serde_lockfile is not None and serde_lockfile.is_file():
        q_lock = shlex.quote(str(serde_lockfile))
        q_dst = shlex.quote(str(repo_dir / "Cargo.lock"))
        binds.append(f"mount --bind {q_lock} {q_dst}")
    q_tdir = shlex.quote(str(target_dir))
    q_repo_t = shlex.quote(str(repo_dir / "target"))
    binds.extend([f"mount --bind {q_tdir} {q_repo_t}", f"cd {shlex.quote(str(repo_dir))}", inner_cmd])
    cargo_bin = str(Path.home() / ".cargo" / "bin")
    path_val = f"{cargo_bin}:{os.environ.get('PATH', '')}"
    env = {**os.environ, "PATH": path_val, "RIDX_CACHE_DIR": str(cache_dir), "CARGO_BUILD_RUSTC_WRAPPER": "", "RUSTC_WRAPPER": ""}
    if ridx_bin is not None:
        sibling_driver = ridx_bin.resolve().parent / "ridx-driver"
        if sibling_driver.is_file():
            env["RIDX_DRIVER_BIN"] = str(sibling_driver)
    with log_path.open("w", encoding="utf-8") as log_f:
        proc = subprocess.run(
            ["unshare", "-rm", "bash", "-c", " && ".join(binds)],
            stdout=log_f, stderr=subprocess.STDOUT, env=env, timeout=timeout_s, check=False,
        )
    return proc.returncode


def index_corpora(
    repos_dir: Path, target_root: Path, cache_dir: Path, ridx_bin: Path,
    builder: str, serde_lockfile: Path | None, out_dir: Path,
) -> None:
    shutil.rmtree(target_root, ignore_errors=True)
    shutil.rmtree(cache_dir, ignore_errors=True)
    target_root.mkdir(parents=True, exist_ok=True)
    cache_dir.mkdir(parents=True, exist_ok=True)
    idx_logs = out_dir / "idx_logs"
    idx_logs.mkdir(parents=True, exist_ok=True)
    for corpus in CORPORA:
        tdir = target_root / corpus
        lock = serde_lockfile if corpus == "serde" else None
        cmd = f"{shlex.quote(str(ridx_bin))} index . --builder {shlex.quote(builder)}"
        rc = run_unshare_cmd(repos_dir / corpus, tdir, cache_dir, cmd, lock, idx_logs / f"{corpus}.log", ridx_bin=ridx_bin)
        manifest = tdir / "ridx" / "ridx_manifest.json"
        if not manifest.is_file():
            raise RuntimeError(f"indexing {corpus} failed (rc={rc}); missing {manifest}")
        print(f"[index] {corpus}: ok (rc={rc})", flush=True)


def run_parity_groups(
    repos_dir: Path, review_dir: Path, target_root: Path, cache_dir: Path,
    ridx_bin: Path, serde_lockfile: Path | None, out_dir: Path,
) -> None:
    bench_py = Path(__file__).resolve().parent / "position_ab_parity.py"
    out_dir.mkdir(parents=True, exist_ok=True)
    for gid, methods in METHOD_GROUPS.items():
        ra_dir = review_dir / ra_cache_subdir(gid)
        print(f"[parity] group {gid}: {methods}", flush=True)
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
            futs = {}
            for corpus in CORPORA:
                pos = resolve_positions_file(review_dir, corpus)
                pkl = ra_dir / f"{corpus}.json.raw.pkl"
                out_json = out_dir / f"{corpus}_g{gid}.json"
                log_path = out_dir / f"{corpus}_g{gid}.log"
                lock = serde_lockfile if corpus == "serde" else None
                cmd = (
                    f"python3 {shlex.quote(str(bench_py))} --repo . "
                    f"--positions {shlex.quote(str(pos))} --ridx ridx1={shlex.quote(str(ridx_bin))} "
                    f"--ra ra=rust-analyzer --methods {shlex.quote(methods)} "
                    f"--ra-cache {shlex.quote(str(pkl))} --out {shlex.quote(str(out_json))}"
                )
                fut = pool.submit(run_unshare_cmd, repos_dir / corpus, target_root / corpus, cache_dir, cmd, lock, log_path, ridx_bin)
                futs[fut] = (corpus, out_json)
            for fut, (corpus, out_json) in futs.items():
                rc = fut.result()
                if not out_json.is_file():
                    raise RuntimeError(f"parity group {gid} failed on {corpus} (rc={rc})")


def aggregate_summary(out_dir: Path) -> dict[str, Any]:
    methods_agg: dict[str, dict[str, Any]] = {}
    per_corpus: dict[str, dict[str, dict[str, int]]] = {c: {} for c in CORPORA}
    index_shards_by_corpus: dict[str, int] = {}
    for corpus in CORPORA:
        for gid in METHOD_GROUPS:
            p = out_dir / f"{corpus}_g{gid}.json"
            if not p.is_file():
                continue
            data = json.loads(p.read_text(encoding="utf-8"))
            if gid == 1 and "index_shards" in data:
                index_shards_by_corpus[corpus] = int(data["index_shards"])
            for m, st in data.get("summary", {}).get("methods", {}).items():
                slot = methods_agg.setdefault(
                    m,
                    {
                        "n": 0,
                        "ra_non_empty": 0,
                        "matched_on_ra_non_empty": 0,
                        "ridx_refusal_justified": 0,
                        "adjusted_matched_on_ra_non_empty": 0,
                        "exact_match": 0,
                        "core_match": 0,
                        "partial": 0,
                        "ridx_only": 0,
                        "both_empty": 0,
                        "mismatch": 0,
                    },
                )
                for k in list(slot.keys()):
                    slot[k] += int(st.get(k, 0))
                per_corpus[corpus][m] = {
                    "ra_non_empty": int(st.get("ra_non_empty", 0)),
                    "adjusted_matched_on_ra_non_empty": int(
                        st.get("adjusted_matched_on_ra_non_empty", 0)
                    ),
                }
    for m, slot in methods_agg.items():
        ra_ne = slot["ra_non_empty"]
        adj = slot["adjusted_matched_on_ra_non_empty"]
        raw_m = slot["matched_on_ra_non_empty"]
        slot["matched_on_ra_non_empty_pct"] = (
            round(100.0 * raw_m / ra_ne, 2) if ra_ne else 100.0
        )
        slot["adjusted_on_ra_non_empty_pct"] = (
            round(100.0 * adj / ra_ne, 2) if ra_ne else 100.0
        )
    summary = {
        "corpora": CORPORA,
        "index_shards_by_corpus": index_shards_by_corpus,
        "method_count": len(methods_agg),
        "methods": dict(sorted(methods_agg.items())),
        "per_corpus": per_corpus,
    }
    (out_dir / "summary.json").write_text(
        json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return summary


def compare_dirs(dir_a: Path, dir_b: Path) -> int:
    sum_a = aggregate_summary(dir_a)
    sum_b = aggregate_summary(dir_b)
    md5_a = hashlib.md5((dir_a / "summary.json").read_bytes()).hexdigest()
    md5_b = hashlib.md5((dir_b / "summary.json").read_bytes()).hexdigest()
    print(f"Base summary.json MD5: {md5_a}")
    print(f"Head summary.json MD5: {md5_b}")
    print(f"Base index_shards: {sum_a.get('index_shards_by_corpus', {})}")
    print(f"Head index_shards: {sum_b.get('index_shards_by_corpus', {})}")
    all_methods = sorted(set(sum_a["methods"]) | set(sum_b["methods"]))
    print(f"Comparing {len(all_methods)} methods across {len(CORPORA)} corpora:")
    print(
        f"{'Method':<28} | {'Base (adj/ra_ne)':<22} | "
        f"{'Head (adj/ra_ne)':<22} | {'Delta':<8}"
    )
    print("-" * 88)
    regressions = 0
    for m in all_methods:
        sa = sum_a["methods"].get(m, {})
        sb = sum_b["methods"].get(m, {})
        ra_a = sa.get("ra_non_empty", 0)
        adj_a = sa.get("adjusted_matched_on_ra_non_empty", 0)
        pct_a = sa.get("adjusted_on_ra_non_empty_pct", 100.0)
        ra_b = sb.get("ra_non_empty", 0)
        adj_b = sb.get("adjusted_matched_on_ra_non_empty", 0)
        pct_b = sb.get("adjusted_on_ra_non_empty_pct", 100.0)
        diff = adj_b - adj_a
        if diff < 0:
            regressions += 1
        ca = f"{pct_a:6.2f}% ({adj_a}/{ra_a})"
        cb = f"{pct_b:6.2f}% ({adj_b}/{ra_b})"
        ds = f"{diff:+d}" if diff != 0 else "0"
        print(f"{m:<28} | {ca:<22} | {cb:<22} | {ds:<8}")
    print("-" * 88)
    print(f"Total regressions: {regressions}")
    return 1 if regressions > 0 else 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--repos-dir", type=Path, help="Path to bench_repos directory")
    ap.add_argument("--review-dir", type=Path, help="Path to review ra_* caches")
    ap.add_argument("--target-root", type=Path, help="Per-corpus target bind dir")
    ap.add_argument("--cache-dir", type=Path, help="RIDX_CACHE_DIR path")
    ap.add_argument("--ridx-bin", type=Path, help="Path to ridx binary")
    ap.add_argument("--out-dir", type=Path, help="Output directory for JSON + summary")
    ap.add_argument("--serde-lockfile", type=Path, default=None)
    ap.add_argument("--builder", default="mitts", choices=["mitts", "cargo"])
    ap.add_argument("--index", action="store_true", help="Index corpora before parity")
    ap.add_argument("--compare", nargs=2, type=Path, metavar=("DIR_A", "DIR_B"))
    args = ap.parse_args()

    if args.compare:
        return compare_dirs(args.compare[0], args.compare[1])

    if not args.out_dir:
        ap.error("--out-dir or --compare is required")
    if args.ridx_bin:
        if not (args.repos_dir and args.review_dir and args.target_root and args.cache_dir):
            ap.error("--repos-dir, --review-dir, --target-root, --cache-dir required")
        if args.index:
            index_corpora(
                args.repos_dir,
                args.target_root,
                args.cache_dir,
                args.ridx_bin,
                args.builder,
                args.serde_lockfile,
                args.out_dir,
            )
        run_parity_groups(
            args.repos_dir,
            args.review_dir,
            args.target_root,
            args.cache_dir,
            args.ridx_bin,
            args.serde_lockfile,
            args.out_dir,
        )
    aggregate_summary(args.out_dir)
    return 0


if __name__ == "__main__":
    sys.exit(main())
