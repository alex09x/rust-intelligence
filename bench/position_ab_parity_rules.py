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
from position_ab_gap_rules import store_kind
from position_ab_parity_norm import normalize_answer
from position_ab_parity_rules_ext import classify_b_to_a_ext, compare_extended_method

NAV_KEYS = frozenset({"definition", "declaration", "type_definition", "implementation"})
REF_KEYS = frozenset({"references", "document_highlight"})
REFS_SUPERSET_MAX_RATIO = 1.25
COMP_RECALL_THRESHOLD = 0.80
COMP_OUTSIDE_MAX = 0.20
REG_VER_RE = re.compile(r"-[0-9]+\.[0-9]+\.[0-9]+[^/]*")


def in_raw_string_literal(lines, idx):
    """True when 0-based line `idx` starts inside a multi-line raw string literal `r"..."`, `br"..."`, or `cr"..."`."""
    if not lines or idx <= 0 or idx >= len(lines):
        return False
    text = "\n".join(lines[:idx])
    i, n = 0, len(text)
    while i < n:
        ch = text[i]
        if ch == "/" and i + 1 < n and text[i + 1] == "/":
            nl = text.find("\n", i + 2)
            i = n if nl < 0 else nl + 1
        elif ch == "/" and i + 1 < n and text[i + 1] == "*":
            depth, i = 1, i + 2
            while i < n and depth > 0:
                if text[i:i + 2] == "/*":
                    depth, i = depth + 1, i + 2
                elif text[i:i + 2] == "*/":
                    depth, i = depth - 1, i + 2
                else:
                    i += 1
        elif ch in ("r", "b", "c") and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] in "_'\"")):
            j = i + (2 if text[i:i + 2] in ("br", "cr") else 1) if ch in ("b", "c") else i + 1
            if ch in ("b", "c") and j == i + 1:
                i += 1
                continue
            hashes = 0
            while j < n and text[j] == "#":
                hashes += 1
                j += 1
            if j < n and text[j] == '"':
                closer = '"' + ("#" * hashes)
                end = text.find(closer, j + 1)
                if end < 0:
                    return True
                i = end + len(closer)
            else:
                i += 1
        elif ch == '"':
            i += 1
            while i < n and text[i] != '"':
                i += 2 if text[i] == "\\" else 1
            i += 1
        elif ch == "'" and i + 2 < n and (text[i + 1] == "\\" or text[i + 2] == "'"):
            i += 4 if text[i + 1] == "\\" and i + 3 < n and text[i + 3] == "'" else 3
        else:
            i += 1
    return False


def cursor_cfg_gated(lines, idx):
    """Returns the enclosing or directly preceding `#[cfg(...)]` / `cfg_*!` gate for 0-based line `idx`."""
    if not lines or idx < 0 or idx >= len(lines):
        return None
    if cfg_context(lines, idx):
        return cfg_context(lines, idx)
    k = idx - 1
    while k >= 0 and (not lines[k].strip() or lines[k].strip().startswith(("#[", "///", "//"))):
        if lines[k].strip().startswith(("#[cfg(", "#![cfg(")):
            return "cfg_attr:" + lines[k].strip()[:60]
        k -= 1
    return None


def in_derive_item(lines, idx, lookback=40):
    """True when line `idx` is on a `#[derive(...)]` attribute or on/inside a derived `struct`/`enum`."""
    if (0 <= idx < len(lines) and "derive" in lines[idx] and lines[idx].lstrip().startswith("#[")) or derive_above(lines, idx, lookback):
        return True
    for j in range(idx - 1, max(-1, idx - lookback), -1):
        if any(lines[j].lstrip().startswith(k) for k in ("struct ", "pub struct ", "enum ", "pub enum ")):
            if block_open_at(lines, j, idx, "{", "}") and derive_above(lines, j, lookback):
                return True
    return False


def is_build_script(path):
    return path == "ws:build.rs" or path.endswith("/build.rs")


def cursor_line_text(row, read_lines_fn):
    lines = read_lines_fn(row["file"]) if read_lines_fn and row.get("file") and row.get("line") else []
    idx = int(row.get("line") or 1) - 1
    return lines[idx] if 0 <= idx < len(lines) else ""


def read_ws_line(ws_path, line_1based, read_lines_fn, fallback_file=None):
    rel = ws_path[len("ws:"):] if ws_path.startswith("ws:") else (fallback_file if ws_path == "doc:" else "")
    lines = read_lines_fn(rel) if read_lines_fn and rel else []
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
    if at and at.issubset(rt):
        cls = "c_nav_superset_of_ra_targets" if len(rt) <= 1.25 * len(at) else "p_nav_ridx_superset_excess"
        return ("core_match" if cls.startswith("c_") else "partial"), cls, {"ridx": sorted(rt), "ra": sorted(at)}
    full = a_norm["full_ranges"]
    if rt and len(rt) == len(at) and all(any(p == fp and fs <= ln <= fe for fp, fs, fe in full) for p, ln in rt):
        return "core_match", "c_nav_within_ra_target_range", {"ridx": sorted(rt), "ra_ranges": full}
    if rt & at:
        return "partial", "p_nav_partial_target_overlap", {"ridx": sorted(rt), "ra": sorted(at), "common": sorted(rt & at)}
    sym = row.get("symbol") or ""
    r_files, a_files = {p for p, _ in rt}, {p for p, _ in at}
    line_text = cursor_line_text(row, read_lines_fn)
    after_tok = suffix_after_cursor_symbol(line_text, int(row.get("character") or 0), sym).lstrip()
    if {(REG_VER_RE.sub("", p), ln) for p, ln in rt} == {(REG_VER_RE.sub("", p), ln) for p, ln in at}:
        return "mismatch", "p_nav_registry_crate_version_skew", {"ridx": sorted(rt), "ra": sorted(at)}
    if after_tok.startswith("!"):
        return "mismatch", "p_nav_macro_call_target", {"ridx": sorted(rt), "ra": sorted(at)}
    if any(p.endswith(f"/{sym}.rs") and ln == 1 for p, ln in at):
        return "mismatch", "p_nav_module_decl_vs_file", {"ridx": sorted(rt), "ra": sorted(at)}
    if (f"ws:{row.get('file')}", row.get("line")) in rt:
        return "mismatch", "p_nav_self_use_or_reexport", {"ridx": sorted(rt), "ra": sorted(at)}
    if row.get("cursor_kind") == "method_call" or after_tok.startswith("(") or any(p.startswith("sysroot:") for p in r_files | a_files):
        return "mismatch", "p_nav_trait_vs_impl_target", {"ridx": sorted(rt), "ra": sorted(at)}
    lines = read_lines_fn(row["file"]) if read_lines_fn and row.get("file") else []
    if lines and row.get("line") and in_derive_item(lines, int(row["line"]) - 1):
        return "mismatch", "p_nav_derive_macro_expansion", {"ridx": sorted(rt), "ra": sorted(at), "in_derive_item": True}
    if row["method"] in ("type_definition", "implementation", "declaration"):
        return "mismatch", f"p_{row['method']}_different_target", {"ridx": sorted(rt), "ra": sorted(at)}
    return "mismatch", "unclassified", {"ridx": sorted(rt), "ra": sorted(at)}


def compare_references(row, r_norm, a_norm, read_lines_fn=None):
    rs, as_ = set(r_norm["spans"]), set(a_norm["spans"])
    rw = active_ws_lines(r_norm["ws_lines"], read_lines_fn)
    aw = active_ws_lines(a_norm["ws_lines"], read_lines_fn)
    common = rw & aw
    if rs == as_:
        return "exact_match", None, {"spans": len(rs), "ridx_ws": len(rw), "ra_ws": len(aw), "common_ws": len(common), "row_precision": 1.0, "row_recall": 1.0}
    if rw == aw and rw:
        return "core_match", "c_refs_active_ws_lines_equal", {"ws_lines": len(rw), "ridx_ws": len(rw), "ra_ws": len(aw), "common_ws": len(rw), "row_precision": 1.0, "row_recall": 1.0}
    if aw and aw.issubset(rw):
        prec = round(len(aw) / len(rw), 4)
        if len(rw) <= REFS_SUPERSET_MAX_RATIO * len(aw) or row["method"] == "document_highlight":
            return "core_match", "c_refs_active_ws_superset", {"ridx_ws": len(rw), "ra_ws": len(aw), "common_ws": len(aw), "row_precision": prec, "row_recall": 1.0}
        return "partial", "p_refs_ridx_excess", {"ridx_ws": len(rw), "ra_ws": len(aw), "common_ws": len(aw), "excess_ratio": round(len(rw) / len(aw), 4), "row_precision": prec, "row_recall": 1.0}
    if row["method"] == "document_highlight" and common and (rw.issubset(aw) or len(common) / min(len(rw), len(aw)) >= 0.50):
        return "core_match", "c_refs_active_ws_superset", {"ridx_ws": len(rw), "ra_ws": len(aw), "common_ws": len(common), "row_precision": round(len(common) / len(rw), 4), "row_recall": round(len(common) / len(aw), 4)}
    if rw and not aw:
        return "ridx_only", "c_refs_ridx_active_ws_ra_none", {"ridx_ws": len(rw), "ra_ws": 0, "common_ws": 0}
    if common:
        ra_only = sorted(aw - rw)
        sym = row.get("symbol") or ""
        all_lines = [read_ws_line(p, ln, read_lines_fn, row.get("file")) for p, ln in ra_only]
        ra_only_total = len(ra_only)
        sym_pat = re.compile(rf"\b{re.escape(sym)}\b") if sym else None
        decl_pat = re.compile(rf"\b(?:fn|type|const)\s+(?:derive_)?{re.escape(sym)}\b") if sym else None
        decl_lines = sum(1 for s in all_lines if decl_pat and decl_pat.search(s))
        macro_lines = sum(1 for s in all_lines if "$(" in s)
        self_kw_lines = sum(1 for s in all_lines if sym_pat and re.search(r"\bSelf\b", s) and not sym_pat.search(s))
        mod_seg_lines = sum(1 for s in all_lines if sym and f"{sym}::" in s)
        non_decl_lines = sum(1 for s in all_lines if s and not (decl_pat and decl_pat.search(s)))
        ok = ra_only_total > 0 and all(all_lines)
        if ok and macro_lines == ra_only_total:
            cls = "p_refs_ra_only_macro_def"
        elif ok and self_kw_lines == ra_only_total:
            cls = "p_refs_ra_only_self_keyword"
        elif ok and decl_lines == ra_only_total:
            cls = "p_refs_trait_impl_sibling_decl"
        elif ok and mod_seg_lines == ra_only_total:
            cls = "p_refs_module_path_segment"
        elif ok and 0 < decl_lines < ra_only_total and non_decl_lines > 0 and decl_lines + non_decl_lines == ra_only_total:
            cls = "p_refs_partial_mixed_decl_and_usages"
        elif ok and decl_lines == 0 and non_decl_lines == ra_only_total:
            cls = "p_refs_partial_overlap"
        else:
            cls = "unclassified"
        return "partial", cls, {
            "common_ws": len(common), "ridx_ws": len(rw), "ra_ws": len(aw),
            "row_precision": round(len(common) / len(rw), 4) if rw else 0.0, "row_recall": round(len(common) / len(aw), 4) if aw else 0.0,
            "ra_only_total": ra_only_total, "decl_lines": decl_lines, "non_decl_lines": non_decl_lines,
            "decl_ratio": round(decl_lines / ra_only_total, 4) if ra_only_total else 0.0,
            "self_keyword_lines": self_kw_lines, "macro_lines": macro_lines, "ra_only_sample": ra_only[:5],
        }
    base_ev = {"ridx_ws": len(rw), "ra_ws": len(aw), "common_ws": 0, "row_precision": 0.0, "row_recall": 0.0}
    if rs and not rw and aw and any(p.startswith(("registry:", "sysroot:", "git:")) for p, *_ in rs):
        return "mismatch", "p_refs_external_decl_only", {**base_ev, "ridx_external": len(rs), "ridx_sample": sorted(rs)[:3]}
    sym = row.get("symbol") or ""
    after_tok = suffix_after_cursor_symbol(cursor_line_text(row, read_lines_fn), int(row.get("character") or 0), sym)
    if rw and aw and (after_tok.startswith("::") or row["method"] == "document_highlight"):
        return "mismatch", "p_refs_path_segment_cursor_line", {**base_ev, "symbol": sym}
    return "mismatch", "unclassified", base_ev


def compare_hover(row, r_norm, a_norm, read_lines_fn=None):
    rd, ad = r_norm["decl"], a_norm["decl"]
    if rd and ad and (
        rd == ad
        or (rd[1] == ad[1] and ({rd[0], ad[0]} <= {"type", "struct", "enum", "union"} or {rd[0], ad[0]} <= {"fn", "proc_macro", "macro_rules!"}))
    ):
        if r_norm["sig_line"] == a_norm["sig_line"]:
            return "exact_match", None, {"sig": r_norm["sig_line"]}
        return "core_match", "c_hover_same_decl_head", {"ridx": r_norm["sig_line"], "ra": a_norm["sig_line"]}
    if not rd and not ad and r_norm["sig_line"] and r_norm["sig_line"] == a_norm["sig_line"]:
        return "exact_match", None, {"sig": r_norm["sig_line"]}
    r_ty = re.sub(r"'[A-Za-z0-9_]+\s*", "", r_norm["type_line"] or r_norm["sig_line"]).strip()
    a_ty = re.sub(r"'[A-Za-z0-9_]+\s*", "", a_norm["type_line"] or a_norm["sig_line"]).strip()
    if not rd and not ad and r_ty and r_ty == a_ty:
        return "core_match", "c_hover_same_decl_head", {"ridx": r_norm["type_line"] or r_norm["sig_line"], "ra": a_norm["sig_line"]}
    if r_norm["sig_line"].startswith("variant ") or (
        not rd and not ad and "::" in r_norm["sig_line"] and "::" in a_norm["sig_line"]
        and r_norm["sig_line"].rsplit("::", 1)[-1] == a_norm["sig_line"].rsplit("::", 1)[-1]
    ):
        return "mismatch", "p_hover_variant_header", {"ridx": r_norm["sig_line"], "ra": a_norm["sig_line"]}
    if r_norm["sig_line"].startswith("macro ") and ad and ad[0] == "macro_rules!":
        return "mismatch", "p_hover_macro_keyword_format", {"ridx": r_norm["sig_line"], "ra": a_norm["sig_line"]}
    if rd and rd[0] == "fn" and ad and ad[0] == "trait":
        return "mismatch", "p_hover_trait_method_container_header", {"ridx_decl": rd, "ra_decl": ad, "ridx": r_norm["sig_line"], "ra": a_norm["sig_line"]}
    if ad and ad[0] == "type" and not rd and r_norm["sig_line"]:
        return "mismatch", "p_hover_assoc_type_decl_format", {"ridx": r_norm["sig_line"], "ra": a_norm["sig_line"]}
    if rd and ad and rd != ad:
        return "mismatch", "p_hover_different_decl_head", {"ridx_decl": rd, "ra_decl": ad}
    if r_norm["type_line"] or r_norm["sig_line"].startswith(("use ", "impl ", "fn ", "field ", "mod ")):
        return "mismatch", "p_hover_enclosing_expr_or_use", {"ridx": r_norm["sig_line"] or r_norm["type_line"], "ra": a_norm["sig_line"]}
    return "mismatch", "unclassified", {"ridx": r_norm, "ra": a_norm}


def compare_completion(row, r_norm, a_norm, read_lines_fn=None):
    rl, al = set(r_norm["labels"]), set(a_norm["labels"])
    sym = row.get("symbol") or ""
    target_in_ra, target_in_ridx = bool(sym and sym in al), bool(sym and sym in rl)
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
        cls = (
            "p_comp_partial_target_present" if target_in_ra and target_in_ridx else
            "p_comp_partial_target_missing" if target_in_ra else
            "p_comp_partial_target_in_ridx_only" if target_in_ridx else "p_comp_partial_no_target"
        )
        return "partial", cls, ev
    line_text = cursor_line_text(row, read_lines_fn)
    if overlap == 0 and ("." in line_text or "::" in line_text or row.get("cursor_kind") in ("method_call", "let", "path", "ident")):
        ev["cursor_line"] = line_text.strip()[:120]
        return "mismatch", "p_comp_assoc_or_unresolved_receiver", ev
    return "mismatch", "unclassified", ev


def prefix_before_cursor_symbol(line_text, col, sym):
    if not line_text:
        return ""
    for m in (re.finditer(rf"\b{re.escape(sym)}\b", line_text) if sym else ()):
        if m.start() <= col <= m.end():
            return line_text[:m.start()]
    c = min(max(0, col), len(line_text))
    while c > 0 and (line_text[c - 1].isalnum() or line_text[c - 1] == "_"):
        c -= 1
    return line_text[:c]


def suffix_after_cursor_symbol(line_text, col, sym):
    if not line_text:
        return ""
    for m in (re.finditer(rf"\b{re.escape(sym)}\b", line_text) if sym else ()):
        if m.start() <= col <= m.end():
            return line_text[m.end():]
    c = min(max(0, col), len(line_text))
    while c < len(line_text) and (line_text[c].isalnum() or line_text[c] == "_"):
        c += 1
    return line_text[c:]


def classify_b_to_a(row, repo, a_res, read_lines_fn=None, ws_table=None):
    """Classifies a row where RIDX answered `b` (empty) and rust-analyzer answered `a`."""
    rel_file = row.get("file") or ""
    if row.get("target_kind") in ("custom-build", "none") or is_build_script("ws:" + rel_file):
        return "p_target_custom_build", {"target_kind": row.get("target_kind"), "file": rel_file}
    if ws_table and rel_file and store_kind(f"{repo.rstrip('/')}/{rel_file}", repo, ws_table) == "ws_unstored":
        return "p_b_to_a_unstored_workspace_file", {"file": rel_file, "target_kind": row.get("target_kind")}
    method = row["method"]
    sym = row.get("symbol") or ""
    lines = read_lines_fn(rel_file) if read_lines_fn and rel_file else []
    idx = int(row.get("line") or 1) - 1
    if in_raw_string_literal(lines, idx):
        return "p_b_to_a_raw_string_fixture", {"file": rel_file, "line": row.get("line"), "symbol": sym}
    a_norm = normalize_answer(method, a_res, repo)
    line_text = lines[idx] if 0 <= idx < len(lines) else ""
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
    cfg = cursor_cfg_gated(lines, idx)
    if cfg and not cfg.endswith("#[cfg(test)]"):
        return "p_b_to_a_cfg_gated_item", {"cfg": cfg, "symbol": sym, "cursor_line": line_text.strip()[:120]}
    if method not in ("definition", "references", "hover", "completion"):
        return classify_b_to_a_ext(row, a_norm, line_text)
    return "unclassified", {"method": method, "symbol": sym, "cursor_line": line_text.strip()[:120]}


def evaluate_row(row, r_ans, a_ans, repo, read_lines_fn=None, ws_table=None):
    """Evaluates one row's RIDX (`r_ans`) and rust-analyzer (`a_ans`) answers."""
    ro, ao = r_ans["outcome"], a_ans["outcome"]
    if ro in ("b", "c") and ao in ("b", "c"):
        if ro == "b" and ao == "b":
            return "both_empty", None, {}
        if ro == "b" and ao == "c":
            return "both_empty", "c_ridx_empty_ra_err", {"ra_error": a_ans.get("error")}
        if ro == "c" and ao == "b":
            return "both_empty", "c_ridx_err_ra_empty", {"ridx_error": r_ans.get("error")}
        return "both_empty", "c_both_error", {"ridx_error": r_ans.get("error"), "ra_error": a_ans.get("error")}
    if ro == "a" and ao in ("b", "c"):
        return "ridx_only", "c_ridx_answers_ra_empty_or_err", {"ridx_outcome": ro, "ra_outcome": ao, "ra_error": a_ans.get("error")}
    if ro in ("b", "c") and ao == "a":
        cls, ev = (("p_ridx_error_or_timeout", {"ridx_error": r_ans.get("error"), "method": row["method"]}) if ro == "c"
                   else classify_b_to_a(row, repo, a_ans.get("res"), read_lines_fn, ws_table))
        if row["method"] in REF_KEYS:
            aw = active_ws_lines(normalize_answer(row["method"], a_ans.get("res"), repo)["ws_lines"], read_lines_fn)
            ev.update({"ridx_ws": 0, "ra_ws": len(aw), "common_ws": 0, "row_precision": 0.0, "row_recall": 0.0})
        elif row["method"] == "completion":
            al = set(normalize_answer("completion", a_ans.get("res"), repo)["labels"])
            sym = row.get("symbol") or ""
            ev.update({"ra_label_recall": 0.0, "ridx_outside_ra_ratio": 0.0, "target_in_ra": bool(sym and sym in al), "target_in_ridx": False, "target_in_top20": False})
        return "mismatch", cls, ev
    method = row["method"]
    r_norm = normalize_answer(method, r_ans.get("res"), repo, legend=r_ans.get("legend"))
    a_norm = normalize_answer(method, a_ans.get("res"), repo, legend=a_ans.get("legend"))
    if method in NAV_KEYS:
        return compare_navigation(row, r_norm, a_norm, read_lines_fn)
    if method in REF_KEYS:
        return compare_references(row, r_norm, a_norm, read_lines_fn)
    if method == "hover":
        return compare_hover(row, r_norm, a_norm, read_lines_fn)
    if method == "completion":
        return compare_completion(row, r_norm, a_norm, read_lines_fn)
    return compare_extended_method(row, r_norm, a_norm, repo=repo, ws_table=ws_table, read_lines_fn=read_lines_fn)
