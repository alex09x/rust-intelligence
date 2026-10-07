#!/usr/bin/env python3
"""
rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
Copyright (c) 2026 Alexander Panasenko

Contact: alex@prod.codes
Author: https://prod.codes/about/
Project: https://github.com/alex09x/rust-intelligence
SPDX-License-Identifier: MIT OR Apache-2.0

Neutral source-level scenario sampling for symmetric LSP benchmarks.
Extracts usage sites and declaration sites directly from Rust source files.
"""

import collections
import random
import re
from pathlib import Path

# Neutral AST / Lexer regexes for Rust source tokens
METHOD_CALL_RE = re.compile(r"\.([a-zA-Z_][a-zA-Z0-9_]*)\s*\(")
PATH_IDENT_RE = re.compile(r"::([a-zA-Z_][a-zA-Z0-9_]*)")
TYPE_USAGE_RE = re.compile(r":\s*([A-Z][a-zA-Z0-9_]*)")
DECL_RE = re.compile(
    r"^\s*(?:pub(?:\([^\)]+\))?\s+)?(?:async\s+)?(fn|struct|enum|trait|type)\s+([a-zA-Z_][a-zA-Z0-9_]*)"
)

KEYWORDS_BLACK_LIST = {
    "fn", "let", "mut", "pub", "async", "await", "match", "if", "else", "loop", "while", "for",
    "in", "return", "break", "continue", "use", "mod", "super", "self", "Self", "crate", "true",
    "false", "where", "impl", "trait", "struct", "enum", "type", "const", "static", "move", "ref", "unsafe", "dyn"
}


def scan_source_positions(repo_dir):
    """Scans all first-party .rs files and extracts usage and declaration sites neutrally."""
    usages = []
    declarations = []
    seen_usages = set()
    seen_decls = set()

    for p in Path(repo_dir).rglob("*.rs"):
        # Ignore build artifacts and hidden directories
        parts = p.parts
        if any(ignored in parts for ignored in ("target", ".git", ".prod", "node_modules")):
            continue
        p_str = str(p.resolve())

        try:
            with open(p, "r", encoding="utf-8", errors="replace") as f:
                in_macro_block = False
                macro_brace_depth = 0
                for line_idx, line in enumerate(f, 1):
                    trimmed = line.strip()
                    if not trimmed or trimmed.startswith("//") or trimmed.startswith("/*") or trimmed.startswith("*"):
                        continue

                    # Filter out macro template bodies (quote!, quote_spanned!, quote_block!, macro_rules!)
                    if any(m in line for m in ("quote!", "quote_spanned!", "quote_block!", "macro_rules!")):
                        in_macro_block = True
                    if in_macro_block:
                        macro_brace_depth += line.count("{") - line.count("}")
                        if macro_brace_depth <= 0:
                            in_macro_block = False
                            macro_brace_depth = 0
                        continue

                    # 1. Declaration sites
                    m_decl = DECL_RE.search(line)
                    if m_decl:
                        name = m_decl.group(2)
                        if name not in KEYWORDS_BLACK_LIST and len(name) > 1:
                            col = line.find(name) + 1
                            if not (col > 1 and line[col - 2] == "#"):
                                key = (p_str, line_idx, col)
                                if key not in seen_decls:
                                    seen_decls.add(key)
                                    declarations.append((p_str, line_idx, col, name, m_decl.group(1)))

                    for regex, u_kind in ((METHOD_CALL_RE, "method_call"), (PATH_IDENT_RE, "path_ident"), (TYPE_USAGE_RE, "type_usage")):
                        for m in regex.finditer(line):
                            name = m.group(1)
                            if name not in KEYWORDS_BLACK_LIST and len(name) > 1:
                                col = m.start(1) + 1
                                if not (col > 1 and line[col - 2] == "#"):
                                    key = (p_str, line_idx, col)
                                    if key not in seen_usages:
                                        seen_usages.add(key)
                                        usages.append((p_str, line_idx, col, name, u_kind))
        except Exception:
            continue

    return usages, declarations


def find_crate_visible_pairs(repo_dir):
    """
    Finds cross-file pairs (d_file, u_file, mod_prefix, u_line, fn_header)
    where d_file is either crate root (src/lib.rs) or a pub mod,
    and u_file is another file in the same crate with a function body to inject probe call.
    """
    pairs = []
    repo_path = Path(repo_dir)

    for cargo_toml in repo_path.rglob("Cargo.toml"):
        parts = cargo_toml.parts
        if any(ignored in parts for ignored in ("target", ".git", ".prod")):
            continue
        crate_dir = cargo_toml.parent
        src_dir = crate_dir / "src"
        if not src_dir.exists():
            continue

        lib_rs = src_dir / "lib.rs"
        main_rs = src_dir / "main.rs"
        root_file = lib_rs if lib_rs.exists() else (main_rs if main_rs.exists() else None)
        if not root_file:
            continue

        visible_defs = [(str(root_file.resolve()), "crate")]

        # Also find any pub mod in root_file
        try:
            with open(root_file, "r", encoding="utf-8", errors="replace") as f:
                content = f.read()
            for m in re.finditer(r"\bpub\s+mod\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*;", content):
                mod_name = m.group(1)
                mod_file = src_dir / f"{mod_name}.rs"
                mod_dir_file = src_dir / mod_name / "mod.rs"
                target_f = mod_file if mod_file.exists() else (mod_dir_file if mod_dir_file.exists() else None)
                if target_f:
                    visible_defs.append((str(target_f.resolve()), f"crate::{mod_name}"))
        except Exception:
            pass

        # Scan other .rs files in the crate for function declarations to call probe
        for rs_file in src_dir.rglob("*.rs"):
            rs_str = str(rs_file.resolve())
            try:
                with open(rs_file, "r", encoding="utf-8", errors="replace") as f:
                    for line_idx, line in enumerate(f, 1):
                        m_fn = re.search(
                            r"(\b(?:pub(?:\([^\)]+\))?\s+)?(?:async\s+)?fn\s+[a-zA-Z_][a-zA-Z0-9_]*\s*(?:<[^>]+>)?\s*\([^\)]*\)[^{]*\{)",
                            line,
                        )
                        if m_fn:
                            fn_header = m_fn.group(1)
                            if "#" in fn_header:
                                continue
                            for def_file, prefix in visible_defs:
                                if def_file != rs_str:
                                    pairs.append({
                                        "d_file": def_file,
                                        "u_file": rs_str,
                                        "mod_prefix": prefix,
                                        "u_line": line_idx,
                                        "fn_header": fn_header,
                                    })
                            break
            except Exception:
                continue

    return pairs


def sample_scenarios(repo_dir, count=300, seed=42):
    """
    Builds a neutral benchmark suite across first-party files:
    - def: strictly on usage sites (call-sites, path identifiers, type annotations)
    - hover: 70% usage sites, 30% declaration sites
    - refs: 50% usage sites, 50% declaration sites
    """
    random.seed(seed)
    usages, declarations = scan_source_positions(repo_dir)

    if not usages and not declarations:
        return []

    target_per_cap = max(10, count // 3)
    scenarios = []

    # 1. Def scenarios: strictly sampled from USAGE sites (call-sites / paths)
    def_candidates = usages if usages else declarations
    sampled_defs = random.sample(def_candidates, min(len(def_candidates), target_per_cap))
    for idx, (f, line, col, name, kind) in enumerate(sampled_defs):
        scenarios.append({"name": f"def_{idx}_{name}", "capability": "def", "target": name, "location": [f, line, col], "kind": kind})

    # 2. Hover scenarios: 70% usages, 30% declarations
    hover_usage_count = int(target_per_cap * 0.7)
    hover_decl_count = target_per_cap - hover_usage_count
    sampled_hover_usages = random.sample(usages, min(len(usages), hover_usage_count)) if usages else []
    sampled_hover_decls = random.sample(declarations, min(len(declarations), hover_decl_count)) if declarations else []

    for idx, (f, line, col, name, kind) in enumerate(sampled_hover_usages + sampled_hover_decls):
        scenarios.append({"name": f"hover_{idx}_{name}", "capability": "hover", "target": name, "location": [f, line, col], "kind": kind})

    # 3. Refs scenarios: 50% usages, 50% declarations
    refs_usage_count = target_per_cap // 2
    refs_decl_count = target_per_cap - refs_usage_count
    sampled_refs_usages = random.sample(usages, min(len(usages), refs_usage_count)) if usages else []
    sampled_refs_decls = random.sample(declarations, min(len(declarations), refs_decl_count)) if declarations else []

    for idx, (f, line, col, name, kind) in enumerate(sampled_refs_usages + sampled_refs_decls):
        scenarios.append({"name": f"refs_{idx}_{name}", "capability": "refs", "target": name, "location": [f, line, col], "kind": kind})

    # 4. Semantic and line-shift cross-file mutations (Point 8 & Review #4751)
    UBIQUITOUS_NAMES = {
        "new", "default", "from", "into", "as_ref", "clone", "fmt", "write", "Result", "Option",
        "Error", "Ok", "Err", "Some", "None", "Value", "Tuple", "Item", "Output", "Target", "Deref",
        "deref", "Serialize", "Deserialize", "Serializer", "Deserializer", "Visitor", "Seed", "len",
        "is_empty", "iter", "next", "map", "and_then", "unwrap", "expect", "is_none", "is_some",
    }
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
        # Guarantees full path resolution (crate::<mod_path>::bench_probe_<n>()) and public visibility
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

    return scenarios
