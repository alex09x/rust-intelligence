#!/usr/bin/env python3
"""
rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
Copyright (c) 2026 Alexander Panasenko

Contact: alex@prod.codes
Author: https://prod.codes/about/
Project: https://github.com/alex09x/rust-intelligence
SPDX-License-Identifier: MIT OR Apache-2.0

Neutral source-level scenario sampling for symmetric LSP benchmarks.
Extracts usage sites, declaration sites, and cross-file mutation scenarios.
"""

import collections
import random
from pathlib import Path
from scan import find_crate_visible_pairs, scan_source_positions

UBIQUITOUS_NAMES = {
    "new", "default", "from", "into", "as_ref", "clone", "fmt", "write", "Result", "Option",
    "Error", "Ok", "Err", "Some", "None", "Value", "Tuple", "Item", "Output", "Target", "Deref",
    "deref", "Serialize", "Deserialize", "Serializer", "Deserializer", "Visitor", "Seed", "len",
    "is_empty", "iter", "next", "map", "and_then", "unwrap", "expect", "is_none", "is_some",
}


def sample_scenarios(repo_dir, count=300, seed=42):
    """
    Builds a neutral benchmark suite across first-party files:
    - def: strictly on usage sites (call-sites, path identifiers, type annotations)
    - hover: 70% usage sites, 30% declaration sites
    - refs: 50% usage sites, 50% declaration sites
    - mutation: cross-file edits with in-scope and out-of-scope boundaries
    """
    random.seed(seed)
    usages, declarations = scan_source_positions(repo_dir)

    if not usages and not declarations:
        return []

    target_per_cap = max(10, count // 3)
    scenarios = []

    # 1. Def queries: strictly on usage sites
    sampled_def_usages = random.sample(usages, min(len(usages), target_per_cap)) if usages else []
    for idx, (f, line, col, name, kind) in enumerate(sampled_def_usages):
        scenarios.append({
            "name": f"def_{idx}_{name}",
            "capability": "def",
            "target": name,
            "location": [f, line, col],
            "kind": kind,
        })

    # 2. Hover queries: 70% usage sites, 30% declaration sites
    hover_usage_count = int(target_per_cap * 0.7)
    hover_decl_count = target_per_cap - hover_usage_count
    sampled_hover_usages = random.sample(usages, min(len(usages), hover_usage_count)) if usages else []
    sampled_hover_decls = random.sample(declarations, min(len(declarations), hover_decl_count)) if declarations else []

    for idx, (f, line, col, name, kind) in enumerate(sampled_hover_usages + sampled_hover_decls):
        scenarios.append({
            "name": f"hover_{idx}_{name}",
            "capability": "hover",
            "target": name,
            "location": [f, line, col],
            "kind": kind,
        })

    # 3. Refs queries: 50% usage sites, 50% declaration sites
    refs_usage_count = target_per_cap // 2
    refs_decl_count = target_per_cap - refs_usage_count
    sampled_refs_usages = random.sample(usages, min(len(usages), refs_usage_count)) if usages else []
    sampled_refs_decls = random.sample(declarations, min(len(declarations), refs_decl_count)) if declarations else []

    for idx, (f, line, col, name, kind) in enumerate(sampled_refs_usages + sampled_refs_decls):
        scenarios.append({
            "name": f"refs_{idx}_{name}",
            "capability": "refs",
            "target": name,
            "location": [f, line, col],
            "kind": kind,
        })

    # 4. Semantic and line-shift cross-file mutations
    ubiquitous_lower = {n.lower() for n in UBIQUITOUS_NAMES}
    decl_counts = collections.Counter(d[3] for d in declarations)
    unique_decls = {
        d[3]: d for d in declarations
        if decl_counts[d[3]] == 1 and d[3].lower() not in ubiquitous_lower and len(d[3]) >= 4
    }
    crate_id_cache = {}

    def _find_crate_id(fp):
        if fp not in crate_id_cache:
            p = Path(fp).resolve()
            cid = None
            for parent in p.parents:
                if (parent / "Cargo.toml").exists():
                    rel = p.relative_to(parent)
                    parts = rel.parts
                    if len(parts) >= 2 and parts[0] == "tests":
                        cid = (str(parent), "test", parts[1].removesuffix(".rs"))
                    elif len(parts) >= 2 and parts[0] == "benches":
                        cid = (str(parent), "bench", parts[1].removesuffix(".rs"))
                    elif parts[0] == "src":
                        cid = (str(parent), "src")
                    else:
                        cid = (str(parent), parts[0])
                    break
            crate_id_cache[fp] = cid
        return crate_id_cache[fp]

    cross_file_pairs = []
    for u in usages:
        u_file, u_line, u_col, u_name, u_kind = u
        if u_name in unique_decls:
            d = unique_decls[u_name]
            d_file, d_line, d_col, _, d_kind = d
            d_cid = _find_crate_id(d_file)
            u_cid = _find_crate_id(u_file)
            if d_file != u_file and d_cid is not None and d_cid == u_cid:
                cross_file_pairs.append((d, u))

    if cross_file_pairs:
        random.shuffle(cross_file_pairs)
        n_pairs = len(cross_file_pairs)

        # 4a. Case A: new function in File A, call in File B (didChange in both without saving)
        visible_pairs = find_crate_visible_pairs(repo_dir)
        if visible_pairs:
            random.shuffle(visible_pairs)
            n_new = min(len(visible_pairs), 30)
            for idx, p in enumerate(visible_pairs[:n_new]):
                d_file = p["d_file"]
                u_file = p["u_file"]
                mod_prefix = p["mod_prefix"]
                u_line = p["u_line"]
                fn_header = p["fn_header"]
                probe_name = f"bench_probe_{idx}"
                target_line = u_line + 1
                target_col = len(f"    let _probe = {mod_prefix}::") + 1
                scenarios.append({
                    "name": f"mutation_new_symbol_{idx}_{probe_name}",
                    "capability": "def",
                    "target": probe_name,
                    "location": [u_file, target_line, target_col],
                    "kind": "new_symbol",
                    "mutation": {
                        "kind": "new_symbol",
                        "file_mutations": [
                            {
                                "file": d_file,
                                "text": f"\npub fn {probe_name}() -> usize {{ 42 }}\n",
                                "is_append": True,
                            },
                            {
                                "file": u_file,
                                "replace_target": fn_header,
                                "replace_with": f"{fn_header}\n    let _probe = {mod_prefix}::{probe_name}();",
                            },
                        ],
                        "expected_target_file": d_file,
                    },
                })
        else:
            n_new = min(max(2, n_pairs // 3), 7)
            for idx, (d, u) in enumerate(cross_file_pairs[:n_new]):
                d_file, d_line, d_col, name, d_kind = d
                u_file, u_line, u_col, _, _ = u
                probe_name = f"bench_probe_{idx}"
                scenarios.append({
                    "name": f"mutation_new_symbol_{idx}_{probe_name}",
                    "capability": "def",
                    "target": probe_name,
                    "location": [u_file, u_line, u_col],
                    "kind": "new_symbol",
                    "mutation": {
                        "kind": "new_symbol",
                        "file_mutations": [
                            {
                                "file": d_file,
                                "text": f"\npub fn {probe_name}() -> usize {{ 42 }}\n",
                                "is_append": True,
                            },
                            {
                                "file": u_file,
                                "replace_target": name,
                                "replace_with": probe_name,
                            },
                        ],
                        "expected_target_file": d_file,
                    },
                })

        # 4b. Case B: rename in File A, call in File B untouched -> stale overlay detector
        pairs_rename = cross_file_pairs[n_new : n_new + 30]
        for idx, (d, u) in enumerate(pairs_rename):
            d_file, d_line, d_col, name, d_kind = d
            u_file, u_line, u_col, _, _ = u
            renamed = f"{name}_renamed_stale_{idx}"
            mut_target = name
            mut_replace = renamed
            try:
                with open(d_file, "r", encoding="utf-8", errors="replace") as f_in:
                    d_lines = f_in.readlines()
                if 1 <= d_line <= len(d_lines):
                    exact_line = d_lines[d_line - 1]
                    if name in exact_line:
                        mut_target = exact_line
                        mut_replace = exact_line.replace(name, renamed, 1)
            except Exception:
                pass

            scenarios.append({
                "name": f"mutation_rename_stale_{idx}_{name}",
                "capability": "def",
                "target": name,
                "location": [u_file, u_line, u_col],
                "kind": "rename_stale",
                "mutation": {
                    "kind": "rename_stale",
                    "file_mutations": [
                        {
                            "file": d_file,
                            "replace_target": mut_target,
                            "replace_with": mut_replace,
                        }
                    ],
                    "expect_not_found": True,
                },
            })

        # 4c. Case C: 10 empty lines prepended in File A -> line displacement
        pairs_shift = cross_file_pairs[n_new + 30 : n_new + 60]
        for idx, (d, u) in enumerate(pairs_shift):
            d_file, d_line, d_col, name, d_kind = d
            u_file, u_line, u_col, _, _ = u
            insert_lines = 10
            scenarios.append({
                "name": f"mutation_line_shift_{idx}_{name}",
                "capability": "def",
                "target": name,
                "location": [u_file, u_line, u_col],
                "kind": "line_shift",
                "mutation": {
                    "kind": "line_shift",
                    "file_mutations": [
                        {
                            "file": d_file,
                            "text": "\n" * insert_lines,
                            "is_append": False,
                        }
                    ],
                    "expected_target_file": d_file,
                    "expected_target_line_delta": insert_lines,
                },
            })

        # 4d. Case D: Out-of-scope boundary 1: method call on existing type in new impl block
        pairs_boundary_method = cross_file_pairs[n_new + 60 : n_new + 75]
        for idx, (d, u) in enumerate(pairs_boundary_method):
            d_file, d_line, d_col, name, d_kind = d
            u_file, u_line, u_col, _, _ = u
            method_probe = f"boundary_method_probe_{idx}"
            scenarios.append({
                "name": f"mutation_out_of_scope_method_{idx}_{name}",
                "capability": "def",
                "target": method_probe,
                "location": [u_file, u_line, u_col],
                "kind": "out_of_scope_method",
                "mutation": {
                    "kind": "out_of_scope_method",
                    "file_mutations": [
                        {
                            "file": d_file,
                            "text": f"\nimpl {name} {{\n    pub fn {method_probe}(&self) -> usize {{ 99 }}\n}}\n",
                            "is_append": True,
                        },
                        {
                            "file": u_file,
                            "replace_target": name,
                            "replace_with": f"{name}.{method_probe}()",
                        },
                    ],
                    "expect_not_found": True,
                },
            })

        # 4e. Case E: Out-of-scope boundary 2: new item imported via `use` statement
        pairs_boundary_use = cross_file_pairs[n_new + 75 : n_new + 90]
        for idx, (d, u) in enumerate(pairs_boundary_use):
            d_file, d_line, d_col, name, d_kind = d
            u_file, u_line, u_col, _, _ = u
            submod_name = f"boundary_submod_{idx}"
            item_name = f"BoundaryItem_{idx}"
            scenarios.append({
                "name": f"mutation_out_of_scope_use_{idx}_{item_name}",
                "capability": "def",
                "target": item_name,
                "location": [u_file, u_line, u_col],
                "kind": "out_of_scope_use",
                "mutation": {
                    "kind": "out_of_scope_use",
                    "file_mutations": [
                        {
                            "file": d_file,
                            "text": f"\npub mod {submod_name} {{\n    pub struct {item_name};\n}}\n",
                            "is_append": True,
                        },
                        {
                            "file": u_file,
                            "text": f"use crate::{submod_name}::{item_name};\n",
                            "is_append": False,
                        },
                    ],
                    "expect_not_found": True,
                },
            })

    return scenarios
