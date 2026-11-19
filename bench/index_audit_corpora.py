#!/usr/bin/env python3
# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0

"""Runs `index_audit --mode sizing` across benchmark corpora and prints Markdown tables in MiB."""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import subprocess
import sys
from typing import Any

MIB = 1024.0 * 1024.0
DEFAULT_CORPORA = [
    "serde",
    "ripgrep",
    "tokio",
    "rust-analyzer",
    "bevy",
    "clap",
    "regex",
]


def mib(n_bytes: int | float) -> str:
    return f"{n_bytes / MIB:.2f}"


def gap_total(g: dict[str, Any]) -> int:
    return sum(
        int(g[k])
        for k in (
            "g1_ty_arena_bytes",
            "g2_fn_sigs_bytes",
            "g3_fields_variants_vals_bytes",
            "g4_generics_bounds_bytes",
            "g5_impls_assoc_bytes",
            "g6_lang_items_bytes",
            "g7_reexports_vis_cfg_bytes",
            "g8_macros_bytes",
        )
    )


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--bin",
        default="target/release/examples/index_audit",
        help="Path to compiled index_audit example binary",
    )
    ap.add_argument(
        "--repos-root",
        default=os.environ.get("RIDX_BENCH_REPOS", "bench_repos"),
        help="Directory containing benchmark repositories (default: $RIDX_BENCH_REPOS or ./bench_repos)",
    )
    ap.add_argument(
        "corpora",
        nargs="*",
        default=DEFAULT_CORPORA,
        help="Corpus directory names or explicit paths",
    )
    args = ap.parse_args()

    paths: list[str] = []
    root = pathlib.Path(args.repos_root)
    for c in args.corpora:
        p = pathlib.Path(c)
        paths.append(str(p if p.exists() else root / c))

    cmd = [args.bin, "--mode", "sizing", *paths]
    proc = subprocess.run(cmd, check=True, capture_output=True, text=True)
    rows = [json.loads(line) for line in proc.stdout.splitlines() if line.strip()]
    if not rows:
        print("No output from index_audit", file=sys.stderr)
        return 1

    print("### 1. Current `.ridx` v4 Disk & Section Breakdown (MiB)\n")
    print(
        "| Corpus | Shards (ws/dep/sys) | Total `.ridx` | WS `.ridx` | Deps `.ridx` | Sysroot | Item Tables (`defs+own+deref`) | Body (`refs+types+notes`) | `string_pool` (of which `sig` strings) |"
    )
    print("|---|---:|---:|---:|---:|---:|---:|---:|---:|")
    for row in rows:
        name = row["corpus"]
        r = row["report"]
        ws, dep, sysr, tot = (
            r["ws_sections"],
            r["dep_sections"],
            r["sysroot_sections"],
            r["total_sections"],
        )
        body_b = tot["body_refs_bytes"] + tot["body_types_notes_bytes"]
        print(
            f"| `{name}` | `{ws['shards']}/{dep['shards']}/{sysr['shards']}` "
            f"| **{mib(tot['disk_bytes'])}** | {mib(ws['disk_bytes'])} | {mib(dep['disk_bytes'])} | {mib(sysr['disk_bytes'])} "
            f"| {mib(tot['item_tables_bytes'])} | {mib(body_b)} "
            f"| {mib(tot['string_pool_bytes'])} ({mib(tot['unique_sig_string_bytes'])}) |"
        )

    print("\n### 2. Phase 1 Gap List & Projected Table Overhead per Corpus (MiB, `ws + deps + sysroot`)\n")
    print(
        "| Corpus | G1 `TyArena` (`1.1`) | G2 `FnSig` (`1.2`) | G3 `Fields+Variants+Aliases` (`1.2`) | G4 `Generics+Where` (`1.2`) | G5 `Impl+AssocTy` (`1.2`) | G6 `LangItems` (`1.2`) | G7 `Use+Vis+Cfg` (`1.3`) | G8 `Macros` (`1.4`) | **Phase 1 Additive** | **Net if `sig` strings dropped** |"
    )
    print("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
    for row in rows:
        name = row["corpus"]
        r = row["report"]
        g = r["total_gaps"]
        tot_s = r["total_sections"]
        add_b = gap_total(g)
        net_b = add_b - int(tot_s["unique_sig_string_bytes"])
        sign = "+" if net_b >= 0 else ""
        print(
            f"| `{name}` | {mib(g['g1_ty_arena_bytes'])} | {mib(g['g2_fn_sigs_bytes'])} "
            f"| {mib(g['g3_fields_variants_vals_bytes'])} | {mib(g['g4_generics_bounds_bytes'])} "
            f"| {mib(g['g5_impls_assoc_bytes'])} | {mib(g['g6_lang_items_bytes'])} "
            f"| {mib(g['g7_reexports_vis_cfg_bytes'])} | {mib(g['g8_macros_bytes'])} "
            f"| **+{mib(add_b)}** | **{sign}{mib(net_b)}** |"
        )

    print("\n### 3. Lang-Item Impl Counts & Macro Call-Site Counts (Measured)\n")
    print(
        "| Corpus | `Deref` | `DerefMut` | `Index/Mut` | `IntoIter/Iter` | `Try` | `Future` | `macro_rules!` defs | WS macro call-sites |"
    )
    print("|---|---:|---:|---:|---:|---:|---:|---:|---:|")
    for row in rows:
        name = row["corpus"]
        li = row["report"]["lang_items"]
        g = row["report"]["total_gaps"]
        print(
            f"| `{name}` | {li['deref_targets']} | {li['deref_mut']} | {li['index_and_mut']} "
            f"| {li['into_iterator_and_iter']} | {li['try_trait']} | {li['future_trait']} "
            f"| {g['macro_defs']} | {g['ws_macro_callsites']} |"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
