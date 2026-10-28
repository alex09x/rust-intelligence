# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Parity comparison and deterministic gap-class assignment for `position_ab_parity.py`.

Every row receives a `parity` outcome (`exact_match`, `core_match`, `partial`, `ridx_only`,
`both_empty`, or `mismatch`) and a deterministic `class` and `evidence` dict (`unclassified` when
no positive predicate proves the difference). No catch-all rule exists: an unproven mismatch or
partial row returns `unclassified`.
"""

import re
from position_ab_gap_cursor import block_open_at, derive_above
from position_ab_gap_reverse import cfg_context
from position_ab_parity_norm import normalize_answer

NAV_KEYS = frozenset({"definition", "declaration", "type_definition", "implementation"})
REF_KEYS = frozenset({"references", "document_highlight"})
COMP_RECALL_THRESHOLD = 0.80
COMP_OUTSIDE_MAX = 0.20


def in_derive_item(lines, idx, lookback=40):
    """True when line `idx` is on or inside a `struct`/`enum` annotated with `#[derive(...)]`."""
    if derive_above(lines, idx, lookback):
        return True
    for j in range(idx - 1, max(-1, idx - lookback), -1):
        stripped = lines[j].lstrip()
        if any(stripped.startswith(k) for k in ("struct ", "pub struct ", "enum ", "pub enum ")):
            if block_open_at(lines, j, idx, "{", "}") and derive_above(lines, j, lookback):
                return True
    return False


def is_build_script(path):
    return path == "ws:build.rs" or path.endswith("/build.rs")


def cursor_line_text(row, read_lines_fn):
    if read_lines_fn is None or not row.get("file") or not row.get("line"):
        return ""
    lines = read_lines_fn(row["file"])
    idx = int(row["line"]) - 1
    return lines[idx] if 0 <= idx < len(lines) else ""


def read_ws_line(ws_path, line_1based, read_lines_fn):
    if read_lines_fn is None or not ws_path.startswith("ws:"):
        return ""
    lines = read_lines_fn(ws_path[len("ws:"):])
    idx = int(line_1based) - 1
    return lines[idx] if 0 <= idx < len(lines) else ""


def active_ws_lines(ws_lines, read_lines_fn=None):
    """Filters workspace `(path, line_1based)` pairs to exclude unindexed `build.rs` files and
    lines inside `#[cfg(windows)]` or `#[cfg(feature = "unstable")]` blocks."""
    out = set()
    for path, line in ws_lines:
        if is_build_script(path):
            continue
        if read_lines_fn is not None and path.startswith("ws:"):
            lines = read_lines_fn(path[len("ws:"):])
            if lines and 0 < line <= len(lines):
                cfg = cfg_context(lines, line - 1) or ""
                if "windows" in cfg or "unstable" in cfg:
                    continue
        out.add((path, line))
    return out


def compare_navigation(row, r_norm, a_norm, read_lines_fn=None):
    rt, at = set(r_norm["targets"]), set(a_norm["targets"])
    if rt == at:
        return "exact_match", None, {"targets": sorted(rt)}
    if rt and rt.issubset(at):
        return "partial", "c_nav_subset_of_ra_targets", {"ridx": sorted(rt), "ra": sorted(at)}
    full = a_norm["full_ranges"]
    if rt and len(rt) == len(at) and all(any(p == fp and fs <= ln <= fe for fp, fs, fe in full) for p, ln in rt):
        return "core_match", "c_nav_within_ra_target_range", {"ridx": sorted(rt), "ra_ranges": full}
    sym = row.get("symbol") or ""
    r_files = {p for p, _ in rt}
    a_files = {p for p, _ in at}
    if any(p.endswith(f"/{sym}.rs") and ln == 1 for p, ln in at):
        return "mismatch", "p_nav_module_decl_vs_file", {"ridx": sorted(rt), "ra": sorted(at)}
    if (f"ws:{row.get('file')}", row.get("line")) in rt:
        return "mismatch", "p_nav_self_use_or_reexport", {"ridx": sorted(rt), "ra": sorted(at)}
    if row.get("cursor_kind") == "method_call" or any(p.startswith("sysroot:") for p in r_files | a_files):
        return "mismatch", "p_nav_trait_vs_impl_target", {"ridx": sorted(rt), "ra": sorted(at)}
    lines = read_lines_fn(row["file"]) if read_lines_fn and row.get("file") else []
    if lines and row.get("line") and in_derive_item(lines, int(row["line"]) - 1):
        return "mismatch", "p_nav_derive_macro_expansion", {"ridx": sorted(rt), "ra": sorted(at), "in_derive_item": True}
    return "mismatch", "unclassified", {"ridx": sorted(rt), "ra": sorted(at)}


def compare_references(row, r_norm, a_norm, read_lines_fn=None):
    rs, as_ = set(r_norm["spans"]), set(a_norm["spans"])
    if rs == as_:
        return "exact_match", None, {"spans": len(rs)}
    rw = active_ws_lines(r_norm["ws_lines"], read_lines_fn)
    aw = active_ws_lines(a_norm["ws_lines"], read_lines_fn)
    if rw == aw and rw:
        return "core_match", "c_refs_active_ws_lines_equal", {"ws_lines": len(rw)}
    if aw and aw.issubset(rw):
        return "core_match", "c_refs_active_ws_superset", {"ridx_ws": len(rw), "ra_ws": len(aw)}
    common = rw & aw
    if common:
        ra_only = sorted(aw - rw)
        sym = row.get("symbol") or ""
        all_lines = [read_ws_line(p, ln, read_lines_fn) for p, ln in ra_only]
        ra_only_total = len(ra_only)
        decl_pat = re.compile(rf"\b(?:fn|type|const)\s+{re.escape(sym)}\b") if sym else None
        decl_lines = sum(1 for s in all_lines if decl_pat and decl_pat.search(s))
        macro_lines = sum(1 for s in all_lines if "$(" in s)
        self_kw_lines = sum(1 for s in all_lines if sym and re.search(r"\bSelf\b", s) and sym not in s)
        mod_seg_lines = sum(1 for s in all_lines if sym and f"{sym}::" in s)
        if ra_only_total > 0 and all(all_lines) and macro_lines == ra_only_total:
            cls = "p_refs_ra_only_macro_def"
        elif ra_only_total > 0 and all(all_lines) and self_kw_lines == ra_only_total:
            cls = "p_refs_ra_only_self_keyword"
        elif ra_only_total > 0 and all(all_lines) and decl_lines == ra_only_total:
            cls = "p_refs_trait_impl_sibling_decl"
        elif ra_only_total > 0 and all(all_lines) and mod_seg_lines == ra_only_total:
            cls = "p_refs_module_path_segment"
        elif decl_lines > 0:
            cls = "p_refs_partial_mixed_decl_and_usages"
        else:
            cls = "p_refs_partial_overlap"
        return "partial", cls, {
            "common_ws": len(common), "ridx_ws": len(rw), "ra_ws": len(aw),
            "ra_only_total": ra_only_total, "decl_lines": decl_lines,
            "decl_ratio": round(decl_lines / ra_only_total, 4) if ra_only_total else 0.0,
            "self_keyword_lines": self_kw_lines, "macro_lines": macro_lines,
            "ra_only_sample": ra_only[:5],
        }
    if rs and not rw and aw and any(p.startswith(("registry:", "sysroot:", "git:")) for p, *_ in rs):
        return "mismatch", "p_refs_external_decl_only", {
            "ridx_external": len(rs), "ra_ws": len(aw), "ridx_sample": sorted(rs)[:3],
        }
    return "mismatch", "unclassified", {"ridx_ws": len(rw), "ra_ws": len(aw)}


def compare_hover(row, r_norm, a_norm):
    if r_norm["decl"] and r_norm["decl"] == a_norm["decl"]:
        if r_norm["sig_line"] == a_norm["sig_line"]:
            return "exact_match", None, {"sig": r_norm["sig_line"]}
        return "core_match", "c_hover_same_decl_head", {"ridx": r_norm["sig_line"], "ra": a_norm["sig_line"]}
    if not r_norm["decl"] and not a_norm["decl"] and r_norm["sig_line"] and r_norm["sig_line"] == a_norm["sig_line"]:
        return "exact_match", None, {"sig": r_norm["sig_line"]}
    if r_norm["sig_line"].startswith("variant "):
        return "mismatch", "p_hover_variant_header", {"ridx": r_norm["sig_line"], "ra": a_norm["sig_line"]}
    if r_norm["decl"] and r_norm["decl"][0] == "fn" and a_norm["decl"] and a_norm["decl"][0] == "trait":
        return "mismatch", "p_hover_trait_method_container_header", {
            "ridx_decl": r_norm["decl"], "ra_decl": a_norm["decl"], "ridx": r_norm["sig_line"], "ra": a_norm["sig_line"],
        }
    if a_norm["decl"] and a_norm["decl"][0] == "type" and not r_norm["decl"] and r_norm["sig_line"]:
        return "mismatch", "p_hover_assoc_type_decl_format", {"ridx": r_norm["sig_line"], "ra": a_norm["sig_line"]}
    if r_norm["decl"] and a_norm["decl"] and r_norm["decl"] != a_norm["decl"]:
        return "mismatch", "p_hover_different_decl_head", {"ridx_decl": r_norm["decl"], "ra_decl": a_norm["decl"]}
    if r_norm["type_line"] or r_norm["sig_line"].startswith(("use ", "impl ", "fn ")):
        return "mismatch", "p_hover_enclosing_expr_or_use", {
            "ridx": r_norm["sig_line"] or r_norm["type_line"], "ra": a_norm["sig_line"],
        }
    return "mismatch", "unclassified", {"ridx": r_norm, "ra": a_norm}


def compare_completion(row, r_norm, a_norm, read_lines_fn=None):
    rl, al = set(r_norm["labels"]), set(a_norm["labels"])
    sym = row.get("symbol") or ""
    target_in_ra = bool(sym and sym in al)
    target_in_ridx = bool(sym and sym in rl)
    target_in_top20 = bool(sym and sym in set(r_norm["top_n"]))
    overlap = len(rl & al)
    ra_recall = (overlap / len(al)) if al else 1.0
    ridx_outside = (len(rl - al) / len(rl)) if rl else 0.0
    ev = {
        "symbol": sym, "overlap": overlap, "ridx_count": len(rl), "ra_count": len(al),
        "ra_label_recall": round(ra_recall, 4), "ridx_outside_ra_ratio": round(ridx_outside, 4),
        "target_in_ra": target_in_ra, "target_in_ridx": target_in_ridx, "target_in_top20": target_in_top20,
    }
    if rl == al:
        return "exact_match", None, ev
    if al and rl and ra_recall >= COMP_RECALL_THRESHOLD and ridx_outside <= COMP_OUTSIDE_MAX and (not target_in_ra or target_in_ridx):
        return "core_match", "c_comp_label_set_parity", ev
    if overlap > 0:
        if target_in_ra and target_in_ridx:
            return "partial", "p_comp_partial_target_present", ev
        if target_in_ra and not target_in_ridx:
            return "partial", "p_comp_partial_target_missing", ev
        if not target_in_ra and target_in_ridx:
            return "partial", "p_comp_partial_target_in_ridx_only", ev
        return "partial", "p_comp_partial_no_target", ev
    line_text = cursor_line_text(row, read_lines_fn)
    if overlap == 0 and ("." in line_text or "::" in line_text or row.get("cursor_kind") in ("method_call", "let", "path", "ident")):
        ev["cursor_line"] = line_text.strip()[:120]
        return "mismatch", "p_comp_assoc_or_unresolved_receiver", ev
    return "mismatch", "unclassified", ev


def prefix_before_cursor_symbol(line_text, col, sym):
    """Returns the line prefix immediately before the identifier token covering 0-based `col`."""
    if not line_text:
        return ""
    if sym:
        for m in re.finditer(rf"\b{re.escape(sym)}\b", line_text):
            if m.start() <= col <= m.end():
                return line_text[:m.start()]
    c = min(max(0, col), len(line_text))
    while c > 0 and (line_text[c - 1].isalnum() or line_text[c - 1] == "_"):
        c -= 1
    return line_text[:c]


def classify_b_to_a(row, repo, a_res, read_lines_fn=None):
    """Classifies a row where RIDX answered `b` (empty) and rust-analyzer answered `a`."""
    if row.get("target_kind") in ("custom-build", "none") or is_build_script("ws:" + (row.get("file") or "")):
        return "p_target_custom_build", {"target_kind": row.get("target_kind"), "file": row.get("file")}
    method = row["method"]
    sym = row.get("symbol") or ""
    a_norm = normalize_answer(method, a_res, repo)
    line_text = cursor_line_text(row, read_lines_fn)
    stripped = line_text.lstrip()
    if stripped.startswith(("use ", "pub use ", "pub(crate) use ", "pub(super) use ")) or row.get("cursor_kind") == "use":
        return "p_b_to_a_use_path_segment", {"symbol": sym, "cursor_line": line_text.strip()[:120]}
    if method == "hover":
        decl = a_norm.get("decl")
        text = a_norm.get("text") or ""
        if "clippy::" in text or "diagnostic" in text or stripped.startswith("#"):
            return "p_b_to_a_attribute_or_lint", {"ra_sig": a_norm.get("sig_line"), "cursor_line": line_text.strip()[:120]}
        if decl and decl[0] == "mod":
            return "p_b_to_a_module_segment", {"ra_decl": decl, "ra_sig": a_norm.get("sig_line")}
    before_tok = prefix_before_cursor_symbol(line_text, int(row.get("character") or 0), sym)
    if before_tok.endswith("Self::"):
        return "p_b_to_a_self_assoc_path", {"symbol": sym, "cursor_line": line_text.strip()[:120]}
    if re.search(r"(?:[A-Z][A-Za-z0-9_]*|>)::$", before_tok):
        return "p_b_to_a_type_relative_assoc_path", {"symbol": sym, "cursor_line": line_text.strip()[:120]}
    if before_tok.endswith("::") or "::" in line_text:
        return "p_b_to_a_qualified_path_cursor", {"symbol": sym, "cursor_line": line_text.strip()[:120]}
    return "unclassified", {"method": method, "symbol": sym, "cursor_line": line_text.strip()[:120]}


def evaluate_row(row, r_ans, a_ans, repo, read_lines_fn=None):
    """Evaluates one row's RIDX (`r_ans`) and rust-analyzer (`a_ans`) answers.
    Returns `(parity, cls, evidence)`."""
    ro, ao = r_ans["outcome"], a_ans["outcome"]
    if ro == "b" and ao == "b":
        return "both_empty", None, {}
    if ro == "a" and ao in ("b", "c"):
        return "ridx_only", "c_ridx_answers_ra_empty_or_err", {"ridx_outcome": ro, "ra_outcome": ao, "ra_error": a_ans.get("error")}
    if ro in ("b", "c") and ao == "a":
        cls, ev = classify_b_to_a(row, repo, a_ans.get("res"), read_lines_fn)
        return "mismatch", cls, ev
    if ro == "c" and ao == "c":
        return "both_empty", "c_both_error", {"ridx_error": r_ans.get("error"), "ra_error": a_ans.get("error")}
    method = row["method"]
    r_norm = normalize_answer(method, r_ans.get("res"), repo)
    a_norm = normalize_answer(method, a_ans.get("res"), repo)
    if method in NAV_KEYS:
        return compare_navigation(row, r_norm, a_norm, read_lines_fn)
    if method in REF_KEYS:
        return compare_references(row, r_norm, a_norm, read_lines_fn)
    if method == "hover":
        return compare_hover(row, r_norm, a_norm)
    if method == "completion":
        return compare_completion(row, r_norm, a_norm, read_lines_fn)
    if r_norm == a_norm:
        return "exact_match", None, {}
    return "mismatch", "unclassified", {"method": method}
