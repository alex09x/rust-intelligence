#!/usr/bin/env python3
"""
rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
Copyright (c) 2026 Alexander Panasenko

Contact: alex@prod.codes
Author: https://prod.codes/about/
Project: https://github.com/alex09x/rust-intelligence
SPDX-License-Identifier: MIT OR Apache-2.0

Source-level scanning and visible pair extraction for benchmark scenario sampling.
"""

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

                    # Filter out macro template bodies
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
