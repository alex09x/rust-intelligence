# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Extended method comparators and b->a classifiers for `position_ab_parity_rules.py`."""

import re
from position_ab_gap_rules import store_kind

FN_NAME_RE = re.compile(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)\b")
PUB_PREFIX_RE = re.compile(r"^\s*pub(?:\([^)]*\))?\s+")


def _sig_fn_name(sig):
    m = FN_NAME_RE.search(sig or "")
    return m.group(1) if m else (sig or "").split("(", 1)[0].strip()


def compare_signature_help(r_norm, a_norm):
    rs, as_ = r_norm["signatures"], a_norm["signatures"]
    if r_norm == a_norm:
        return "exact_match", None, {"sig": rs[0] if rs else ""}
    r_head = PUB_PREFIX_RE.sub("", rs[0]) if rs else ""
    a_head = PUB_PREFIX_RE.sub("", as_[0]) if as_ else ""
    if r_head and r_head == a_head and r_norm["active_param"] == a_norm["active_param"]:
        return "core_match", "c_sig_same_fn_and_active_param", {"ridx": rs[:1], "ra": as_[:1], "active_param": r_norm["active_param"]}
    r_fn, a_fn = _sig_fn_name(r_head), _sig_fn_name(a_head)
    if r_fn and r_fn == a_fn and r_norm["arity"] == a_norm["arity"]:
        if r_norm["active_param"] == a_norm["active_param"]:
            return "core_match", "c_sig_same_callee_arity_and_param", {"ridx": rs[:1], "ra": as_[:1], "arity": r_norm["arity"], "active_param": r_norm["active_param"]}
        return "partial", "p_sig_same_callee_active_param_diff", {"ridx": rs[:1], "ra": as_[:1], "ridx_param": r_norm["active_param"], "ra_param": a_norm["active_param"]}
    if r_fn and r_fn == a_fn:
        return "partial", "p_sig_same_callee_arity_diff", {"ridx": rs[:1], "ra": as_[:1], "ridx_arity": r_norm["arity"], "ra_arity": a_norm["arity"]}
    return "mismatch", "p_sig_different_callee", {"ridx": rs[:1], "ra": as_[:1]}


def compare_inlay_hint(r_norm, a_norm):
    rh, ah = set(r_norm["hints"]), set(a_norm["hints"])
    rl, al = set(r_norm["line_labels"]), set(a_norm["line_labels"])
    rtl, atl = set(r_norm.get("type_line_labels", [])), set(a_norm.get("type_line_labels", []))
    if not rh and not ah:
        return "both_empty", None, {}
    if rh == ah:
        return "exact_match", None, {"count": len(rh)}
    if rl == al and rl:
        return "core_match", "c_inlay_same_line_labels", {"count": len(rl)}
    if rtl and rtl == atl:
        return "core_match", "c_inlay_same_type_hints", {"type_hints": len(rtl), "ra_all": len(al)}
    if rtl and atl and (rtl & atl) and len(rtl & atl) / min(len(rtl), len(atl)) >= 0.50:
        return "core_match", "c_inlay_type_hint_overlap", {"ridx_type": len(rtl), "ra_type": len(atl), "common": len(rtl & atl)}
    common_lines = {ln for ln, _ in rl} & {ln for ln, _ in al}
    if (rl & al) or (rtl & atl):
        return "partial", "p_inlay_partial_label_overlap", {"ridx": len(rl), "ra": len(al), "common": len(rl & al)}
    if common_lines:
        return "partial", "p_inlay_same_line_diff_label_format", {"ridx": len(rl), "ra": len(al), "common_lines": len(common_lines)}
    if not rl and al and not atl:
        return "partial", "p_inlay_ra_non_type_hints_only", {"ra_non_type": len(al)}
    return "mismatch", "p_inlay_disjoint_hint_positions", {"ridx": len(rh), "ra": len(ah)}


def compare_document_symbol(r_norm, a_norm):
    ra_all, aa_all = set(r_norm["all"]), set(a_norm["all"])
    ra_top, aa_top = set(r_norm["top"]), set(a_norm["top"])
    rn, an = set(r_norm["names"]), set(a_norm["names"])
    if ra_all == aa_all:
        return "exact_match", None, {"count": len(ra_all)}
    if ra_top == aa_top and ra_top:
        return "core_match", "c_docsym_top_level_equal", {"top": len(ra_top), "ridx_all": len(ra_all), "ra_all": len(aa_all)}
    if aa_top and len(ra_top & aa_top) / len(aa_top) >= 0.80:
        return "core_match", "c_docsym_top_level_high_recall", {"ridx_top": len(ra_top), "ra_top": len(aa_top), "common_top": len(ra_top & aa_top)}
    if (ra_all & aa_all) or (rn & an):
        return "partial", "p_docsym_partial_symbol_overlap", {"ridx_all": len(ra_all), "ra_all": len(aa_all), "common_names": len(rn & an)}
    return "mismatch", "p_docsym_disjoint_symbols", {"ridx_all": len(ra_all), "ra_all": len(aa_all)}


def compare_workspace_symbol(row, r_norm, a_norm):
    ri, ai = set(r_norm["items"]), set(a_norm["items"])
    rn, an = set(r_norm["names"]), set(a_norm["names"])
    sym = row.get("symbol") or ""
    if ri == ai:
        return "exact_match", None, {"count": len(ri)}
    r_loc = {(p, nm, ln) for p, nm, _, ln in ri}
    a_loc = {(p, nm, ln) for p, nm, _, ln in ai}
    if r_loc == a_loc and r_loc:
        return "core_match", "c_wssym_same_locations", {"count": len(r_loc)}
    if a_loc and a_loc.issubset(r_loc):
        return "core_match", "c_wssym_superset_of_ra", {"ridx": len(r_loc), "ra": len(a_loc)}
    if sym and sym in rn and sym in an and (r_loc & a_loc):
        return "core_match", "c_wssym_query_target_and_loc_overlap", {"ridx": len(r_loc), "ra": len(a_loc), "common_loc": len(r_loc & a_loc)}
    if (r_loc & a_loc) or (rn & an):
        return "partial", "p_wssym_partial_overlap", {"ridx": len(ri), "ra": len(ai), "common_loc": len(r_loc & a_loc), "common_names": len(rn & an)}
    return "mismatch", "p_wssym_disjoint_results", {"ridx": len(ri), "ra": len(ai)}


def compare_folding_range(r_norm, a_norm):
    ra_all, aa_all = set(r_norm["all"]), set(a_norm["all"])
    rr, ar = set(r_norm["regions"]), set(a_norm["regions"])
    rs, as_ = set(r_norm["spans"]), set(a_norm["spans"])
    if ra_all == aa_all:
        return "exact_match", None, {"count": len(ra_all)}
    if rs == as_ and rs:
        return "core_match", "c_fold_same_line_spans", {"count": len(rs)}
    if rr == ar and rr:
        return "core_match", "c_fold_same_code_regions", {"regions": len(rr), "ridx_all": len(ra_all), "ra_all": len(aa_all)}
    if ar and len(rr & ar) / len(ar) >= 0.80:
        return "core_match", "c_fold_region_high_recall", {"ridx_regions": len(rr), "ra_regions": len(ar), "common": len(rr & ar)}
    if (rs & as_) or (rr & ar):
        return "partial", "p_fold_partial_span_overlap", {"ridx": len(rs), "ra": len(as_), "common_spans": len(rs & as_), "common_regions": len(rr & ar)}
    return "mismatch", "p_fold_disjoint_ranges", {"ridx": len(rs), "ra": len(as_)}


def compare_selection_range(r_norm, a_norm):
    if r_norm == a_norm:
        return "exact_match", None, {"chains": len(r_norm)}
    rc, ac = (r_norm[0] if r_norm else []), (a_norm[0] if a_norm else [])
    if rc and ac:
        if rc[0] == ac[0] and rc[-1] == ac[-1]:
            return "core_match", "c_sel_same_inner_and_outer_bounds", {"inner": rc[0], "outer": rc[-1], "ridx_depth": len(rc), "ra_depth": len(ac)}
        if rc[0] == ac[0] or (set(rc) & set(ac)):
            return "partial", "p_sel_partial_chain_overlap", {"ridx_inner": rc[0], "ra_inner": ac[0], "common_nodes": len(set(rc) & set(ac))}
    return "mismatch", "p_sel_different_bounds", {"ridx": rc[:3], "ra": ac[:3]}


def compare_semantic_tokens(row, r_norm, a_norm, repo=None, ws_table=None):
    r_all = r_norm.get("all_map") or {}
    a_all = a_norm.get("all_map") or {}
    a_sym = a_norm.get("sym_map") or {}
    a_lex = int(a_norm.get("lex_count") or 0)
    shared_all = set(r_all) & set(a_all)
    shared_sym = set(r_all) & set(a_sym)
    span_recall = round(len(shared_sym) / len(a_sym), 4) if a_sym else 1.0
    raw_span_recall = round(len(shared_all) / len(a_all), 4) if a_all else 1.0
    type_agree = round(sum(1 for sp in shared_all if r_all[sp] == a_all[sp]) / len(shared_all), 4) if shared_all else (1.0 if not a_sym and not r_all else 0.0)
    ev = {
        "ridx": len(r_all), "ra_all": len(a_all), "ra_sym": len(a_sym), "ra_lexical_only": a_lex,
        "shared_spans": len(shared_all), "span_recall": span_recall, "raw_span_recall": raw_span_recall, "type_agreement": type_agree,
    }
    if not r_all and not a_all and r_norm["edits"] == a_norm["edits"]:
        return "both_empty", None, ev
    if not r_all and a_all and not a_sym:
        return "core_match", "c_semtok_ra_lexical_only_scope", ev
    if r_all == a_all and r_all:
        return "exact_match", None, ev
    if r_all == a_sym and r_all:
        return "core_match", "c_semtok_exact_symbol_tokens_ra_has_lexical", ev
    if a_sym and span_recall >= 0.65 and type_agree >= 0.75:
        return "core_match", "c_semtok_symbol_span_and_type_parity", ev
    if shared_all:
        cls = "p_semtok_high_type_agree_lower_span_recall" if type_agree >= 0.75 else "p_semtok_partial_token_overlap"
        return "partial", cls, ev
    if r_all and not a_all:
        return "ridx_only", "c_semtok_ridx_tokens_ra_empty", ev
    if a_sym and not r_all:
        rel_file = row.get("file") or ""
        if row.get("target_kind") in ("custom-build", "none") or rel_file == "build.rs" or rel_file.endswith("/build.rs"):
            return "mismatch", "p_target_custom_build", ev
        if ws_table and repo and rel_file and store_kind(f"{repo.rstrip('/')}/{rel_file}", repo, ws_table) == "ws_unstored":
            return "mismatch", "p_b_to_a_unstored_workspace_file", ev
        return "mismatch", "p_semtok_ridx_empty_ra_tokens", ev
    return "mismatch", "p_semtok_disjoint_tokens", ev


def compare_hierarchy_items(r_norm, a_norm):
    ri, ai = set(r_norm["items"]), set(a_norm["items"])
    rn, an = set(r_norm["names"]), set(a_norm["names"])
    if ri == ai:
        return "exact_match", None, {"items": sorted(ri)}
    if rn == an and rn:
        return "core_match", "c_hier_same_path_and_name", {"ridx": sorted(ri), "ra": sorted(ai)}
    full = a_norm["full_ranges"]
    if ri and len(ri) == len(ai) and all(any(p == fp and fs <= ln <= fe for fp, _, fs, fe in full) for p, _, _, ln in ri):
        return "core_match", "c_hier_within_enclosing_item_range", {"ridx": sorted(ri), "ra": sorted(ai)}
    if (ri & ai) or (rn & an):
        return "partial", "p_hier_partial_overlap", {"ridx": sorted(ri), "ra": sorted(ai)}
    return "mismatch", "p_hier_different_target_item", {"ridx": sorted(ri), "ra": sorted(ai)}


def compare_incoming_calls(r_norm, a_norm):
    rc, ac = set(r_norm["callers"]), set(a_norm["callers"])
    rn, an = set(r_norm["caller_names"]), set(a_norm["caller_names"])
    rs, as_ = set(r_norm["call_sites"]), set(a_norm["call_sites"])
    if rc == ac and rs == as_:
        return "exact_match", None, {"callers": len(rc), "sites": len(rs)}
    if rn == an and rn:
        return "core_match", "c_incoming_same_caller_functions", {"callers": len(rn), "ridx_sites": len(rs), "ra_sites": len(as_)}
    if an and an.issubset(rn) and len(rn) <= 1.25 * len(an):
        return "core_match", "c_incoming_caller_superset", {"ridx_callers": len(rn), "ra_callers": len(an)}
    if (rn & an) or (rs & as_):
        return "partial", "p_incoming_partial_caller_overlap", {"ridx_callers": len(rn), "ra_callers": len(an), "common_callers": len(rn & an), "common_sites": len(rs & as_)}
    return "mismatch", "p_incoming_disjoint_callers", {"ridx_callers": len(rn), "ra_callers": len(an)}


def compare_outgoing_calls(r_norm, a_norm):
    rc, ac = set(r_norm["callees"]), set(a_norm["callees"])
    rn, an = set(r_norm["callee_names"]), set(a_norm["callee_names"])
    rl, al = set(r_norm["call_lines"]), set(a_norm["call_lines"])
    if rc == ac:
        return "exact_match", None, {"callees": len(rc)}
    if rn == an and rn:
        return "core_match", "c_outgoing_same_callee_names", {"callees": len(rn)}
    if an and len(rn & an) / len(an) >= 0.80:
        return "core_match", "c_outgoing_high_callee_recall", {"ridx": len(rn), "ra": len(an), "common": len(rn & an)}
    if (rn & an) or (rl & al):
        return "partial", "p_outgoing_partial_callee_overlap", {"ridx": len(rn), "ra": len(an), "common_names": len(rn & an), "common_lines": len(rl & al)}
    return "mismatch", "p_outgoing_disjoint_callees", {"ridx": len(rn), "ra": len(an)}


def compare_prepare_rename(r_norm, a_norm):
    if r_norm == a_norm:
        return "exact_match", None, {"range": r_norm["range"]}
    if r_norm and a_norm and r_norm["range"] == a_norm["range"]:
        return "core_match", "c_prep_rename_same_range", {"range": r_norm["range"], "ridx_placeholder": r_norm["placeholder"], "ra_placeholder": a_norm["placeholder"]}
    if r_norm and a_norm and r_norm["range"][0] == a_norm["range"][0]:
        return "partial", "p_prep_rename_same_line_diff_cols", {"ridx": r_norm, "ra": a_norm}
    return "mismatch", "p_prep_rename_different_range", {"ridx": r_norm, "ra": a_norm}


def compare_rename(r_norm, a_norm):
    re_set, ae_set = set(r_norm), set(a_norm)
    if re_set == ae_set:
        return "exact_match", None, {"edits": len(re_set)}
    rw = {(p, sl) for p, sl, *_ in re_set if p.startswith("ws:") and not p.endswith("/build.rs") and p != "ws:build.rs"}
    aw = {(p, sl) for p, sl, *_ in ae_set if p.startswith("ws:") and not p.endswith("/build.rs") and p != "ws:build.rs"}
    if rw == aw and rw:
        return "core_match", "c_rename_same_ws_lines", {"ws_lines": len(rw), "ridx_edits": len(re_set), "ra_edits": len(ae_set)}
    if aw and aw.issubset(rw) and len(rw) <= 1.25 * len(aw):
        return "core_match", "c_rename_ws_lines_superset", {"ridx_ws": len(rw), "ra_ws": len(aw)}
    if rw & aw:
        return "partial", "p_rename_partial_ws_overlap", {"ridx_ws": len(rw), "ra_ws": len(aw), "common_ws": len(rw & aw)}
    return "mismatch", "p_rename_disjoint_edits", {"ridx_ws": len(rw), "ra_ws": len(aw), "ridx_edits": len(re_set), "ra_edits": len(ae_set)}


def compare_code_action(r_norm, a_norm):
    rk, ak = set(r_norm["kinds"]), set(a_norm["kinds"])
    rt, at = set(r_norm["titles"]), set(a_norm["titles"])
    if rt == at:
        return "exact_match", None, {"titles": sorted(rt)}
    if rk & ak:
        return "partial", "p_code_action_shared_kind_diff_titles", {"ridx_kinds": sorted(rk), "ra_kinds": sorted(ak), "ridx_titles": sorted(rt)[:5], "ra_titles": sorted(at)[:5]}
    return "mismatch", "p_code_action_disjoint_kinds", {"ridx_kinds": sorted(rk), "ra_kinds": sorted(ak), "ridx_titles": sorted(rt)[:5], "ra_titles": sorted(at)[:5]}


def compare_formatting(r_norm, a_norm):
    re_set, ae_set = set(r_norm), set(a_norm)
    if not re_set and not ae_set:
        return "both_empty", None, {}
    if re_set == ae_set:
        return "exact_match", None, {"edits": len(re_set)}
    if len(r_norm) == 1 and len(a_norm) == 1 and r_norm[0][4].strip() == a_norm[0][4].strip():
        return "core_match", "c_fmt_same_formatted_text", {"edits": 1}
    if re_set & ae_set:
        return "partial", "p_fmt_partial_edit_overlap", {"ridx": len(re_set), "ra": len(ae_set), "common": len(re_set & ae_set)}
    return "mismatch", "p_fmt_different_edits", {"ridx": len(re_set), "ra": len(ae_set)}


def compare_diagnostics(r_norm, a_norm):
    rew, aew = set(r_norm["errors_warnings"]), set(a_norm["errors_warnings"])
    rhi, ahi = set(r_norm["hints_info"]), set(a_norm["hints_info"])
    if not rew and not aew and not rhi and not ahi:
        return "exact_match", None, {"errors_warnings": 0, "hints_info": 0}
    if rew == aew and rhi == ahi:
        return "exact_match", None, {"errors_warnings": len(rew), "hints_info": len(rhi)}
    if rew == aew and not rew and ahi:
        return "core_match", "c_diag_both_zero_errors_ra_hints_only", {"ra_hints": len(ahi)}
    r_lines = {(p, ln, sev) for p, ln, sev, _, _ in rew}
    a_lines = {(p, ln, sev) for p, ln, sev, _, _ in aew}
    if r_lines == a_lines and r_lines:
        return "core_match", "c_diag_same_error_warning_lines", {"count": len(r_lines)}
    if r_lines & a_lines:
        return "partial", "p_diag_partial_error_overlap", {"ridx": len(r_lines), "ra": len(a_lines), "common": len(r_lines & a_lines)}
    if rew and not aew:
        return "ridx_only", "c_diag_ridx_only_diagnostics", {"ridx_ew": len(rew)}
    return "mismatch", "p_diag_ra_errors_missed_by_ridx", {"ridx_ew": len(rew), "ra_ew": len(aew), "ra_sample": sorted(aew)[:3]}


def compare_code_lens(r_norm, a_norm):
    rl, al = set(r_norm["lines"]), set(a_norm["lines"])
    rs, as_ = set(r_norm["spans"]), set(a_norm["spans"])
    if rs == as_:
        return "exact_match", None, {"count": len(rs)}
    if rl == al and rl:
        return "core_match", "c_codelens_same_target_lines", {"lines": len(rl)}
    if al and len(rl & al) / len(al) >= 0.80:
        return "core_match", "c_codelens_high_line_recall", {"ridx": len(rl), "ra": len(al), "common": len(rl & al)}
    if rl & al:
        return "partial", "p_codelens_partial_line_overlap", {"ridx": len(rl), "ra": len(al), "common": len(rl & al)}
    return "mismatch", "p_codelens_disjoint_lines", {"ridx": len(rl), "ra": len(al)}


def compare_extended_method(row, r_norm, a_norm, repo=None, ws_table=None):
    method = row["method"]
    if method == "signature_help":
        return compare_signature_help(r_norm, a_norm)
    if method == "inlay_hint":
        return compare_inlay_hint(r_norm, a_norm)
    if method == "document_symbol":
        return compare_document_symbol(r_norm, a_norm)
    if method == "workspace_symbol":
        return compare_workspace_symbol(row, r_norm, a_norm)
    if method == "folding_range":
        return compare_folding_range(r_norm, a_norm)
    if method == "selection_range":
        return compare_selection_range(r_norm, a_norm)
    if method in ("semantic_tokens_full", "semantic_tokens_delta", "semantic_tokens_range"):
        return compare_semantic_tokens(row, r_norm, a_norm, repo=repo, ws_table=ws_table)
    if method in ("prepare_call_hierarchy", "prepare_type_hierarchy", "supertypes", "subtypes"):
        return compare_hierarchy_items(r_norm, a_norm)
    if method == "incoming_calls":
        return compare_incoming_calls(r_norm, a_norm)
    if method == "outgoing_calls":
        return compare_outgoing_calls(r_norm, a_norm)
    if method == "prepare_rename":
        return compare_prepare_rename(r_norm, a_norm)
    if method == "rename":
        return compare_rename(r_norm, a_norm)
    if method == "code_action":
        return compare_code_action(r_norm, a_norm)
    if method in ("formatting", "range_formatting", "on_type_formatting"):
        return compare_formatting(r_norm, a_norm)
    if method in ("document_diagnostic", "workspace_diagnostic"):
        return compare_diagnostics(r_norm, a_norm)
    if method == "code_lens":
        return compare_code_lens(r_norm, a_norm)
    if method == "code_lens_resolve":
        rt, at = r_norm.get("title") or "", a_norm.get("title") or ""
        rr, ar = r_norm.get("range"), a_norm.get("range")
        if not rt and not at:
            return "both_empty", None, {}
        if rt == at and rr == ar:
            return "exact_match", None, {"title": rt, "range": rr}
        if rt == at and rt:
            return "core_match", "c_codelens_resolve_same_title", {"title": rt, "ridx_range": rr, "ra_range": ar}
        if rt and at and rr and ar and rr[0] == ar[0]:
            return "partial", "p_codelens_resolve_same_line_refs_vs_impl_title", {"ridx_title": rt, "ra_title": at, "line": rr[0]}
        if rt and at:
            return "partial", "p_codelens_resolve_diff_lens_title", {"ridx_title": rt, "ra_title": at, "ridx_range": rr, "ra_range": ar}
        if rt and not at:
            return "ridx_only", "c_codelens_resolve_ridx_title_ra_unresolved", {"ridx_title": rt, "ra_range": ar}
        return "mismatch", "p_codelens_resolve_ridx_unresolved", {"ra_title": at}
    if method.endswith("_resolve"):
        if r_norm["raw"] == a_norm["raw"]:
            return "exact_match", None, {"key_id": r_norm["key_id"]}
        if r_norm["key_id"] is not None and r_norm["key_id"] == a_norm["key_id"]:
            return "core_match", "c_resolve_same_item_identity", {"key_id": r_norm["key_id"]}
        return "partial", "p_resolve_different_enriched_fields", {"ridx_key": r_norm["key_id"], "ra_key": a_norm["key_id"]}
    if r_norm == a_norm:
        return "exact_match", None, {}
    return "mismatch", "p_ext_response_diff", {"method": method}


def classify_b_to_a_ext(row, a_norm, line_text):
    """Positive b->a predicates for extended methods (not used for core 4 methods)."""
    method, sym, ck = row["method"], (row.get("symbol") or ""), row.get("cursor_kind")
    if method == "type_definition":
        if any(p.startswith(("sysroot:", "registry:", "git:")) for p, _ in a_norm.get("targets", [])):
            return "p_b_to_a_typedef_external_or_sysroot_type", {"ra_targets": a_norm["targets"][:3], "symbol": sym}
        return "p_b_to_a_typedef_untracked_expr_or_binding", {"ra_targets": a_norm.get("targets", [])[:3], "symbol": sym, "cursor_kind": ck}
    if method == "implementation":
        return "p_b_to_a_impl_macro_or_concrete_type", {"ra_targets": a_norm.get("targets", [])[:3], "symbol": sym, "cursor_kind": ck}
    if method == "declaration":
        return "p_b_to_a_decl_unindexed_or_local", {"ra_targets": a_norm.get("targets", [])[:3], "symbol": sym, "cursor_kind": ck}
    if method == "document_highlight":
        return "p_b_to_a_highlight_local_or_keyword_token", {"ra_spans": len(a_norm.get("spans", [])), "symbol": sym, "cursor_kind": ck}
    if method == "signature_help":
        col = int(row.get("character") or 0)
        cls = "p_b_to_a_sig_cursor_before_open_paren" if "(" not in line_text[:col] else "p_b_to_a_sig_unresolved_or_multiline_call"
        return cls, {"ra_sigs": a_norm.get("signatures", [])[:2], "cursor_line": line_text.strip()[:120]}
    if method == "inlay_hint":
        if not a_norm.get("type_line_labels") and a_norm.get("line_labels"):
            return "p_b_to_a_inlay_ra_non_type_hints_only", {"ra_hints": len(a_norm.get("hints", []))}
        return "p_b_to_a_inlay_no_ridx_heuristic_in_range", {"ra_hints": len(a_norm.get("hints", [])), "ra_type_hints": len(a_norm.get("type_line_labels", []))}
    if method == "code_lens_resolve":
        return "p_b_to_a_codelens_resolve_no_ridx_lens", {"ra_title": a_norm.get("title"), "ra_range": a_norm.get("range")}
    if method == "document_symbol":
        return "p_b_to_a_docsym_unindexed_file", {"ra_symbols": len(a_norm.get("all", [])), "file": row.get("file")}
    if method == "workspace_symbol":
        return "p_b_to_a_wssym_no_ridx_match", {"ra_items": len(a_norm.get("items", [])), "symbol": sym}
    if method == "folding_range":
        return "p_b_to_a_fold_single_line_or_comments_only", {"ra_ranges": len(a_norm.get("all", []))}
    if method == "selection_range":
        return "p_b_to_a_selection_empty_chain", {"ra_chains": len(a_norm)}
    if method in ("prepare_call_hierarchy", "prepare_type_hierarchy", "supertypes", "subtypes"):
        return f"p_b_to_a_{method}_unresolved_item", {"ra_items": a_norm.get("items", [])[:3], "symbol": sym}
    if method == "incoming_calls":
        return "p_b_to_a_incoming_calls_missed_by_ridx", {"ra_callers": a_norm.get("callers", [])[:5], "symbol": sym}
    if method == "outgoing_calls":
        return "p_b_to_a_outgoing_calls_missed_by_ridx", {"ra_callees": a_norm.get("callees", [])[:5], "symbol": sym}
    if method in ("prepare_rename", "rename"):
        return f"p_b_to_a_{method}_unresolved_target", {"symbol": sym, "cursor_kind": ck, "cursor_line": line_text.strip()[:120]}
    if method == "code_action":
        return "p_b_to_a_code_action_ra_refactor_at_cursor", {"ra_kinds": a_norm.get("kinds", []), "ra_count": len(a_norm.get("titles", []))}
    if method in ("formatting", "range_formatting", "on_type_formatting"):
        return f"p_b_to_a_{method}_ra_edits_only", {"ra_edits": len(a_norm)}
    if method == "code_lens":
        return "p_b_to_a_codelens_no_ridx_lens_in_file", {"ra_lenses": len(a_norm.get("spans", []))}
    return "unclassified", {"method": method, "symbol": sym, "cursor_line": line_text.strip()[:120]}
