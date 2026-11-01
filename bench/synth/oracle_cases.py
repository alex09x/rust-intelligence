# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Builds deterministic ground-truth test cases across all 37 LSP methods from a synth manifest."""

from typing import Dict, List


def _span_key(rec: dict) -> tuple:
    return (rec["rel_path"], rec["line"], rec["col"], rec["end_line"], rec["end_col"])


def _first_role(symbols: Dict[str, List[dict]], sym_id: str, role: str) -> dict:
    for r in symbols.get(sym_id, ()):
        if r["role"] == role:
            return r
    raise KeyError(f"Missing role={role!r} for symbol={sym_id!r}")


def _all_keys(symbols: Dict[str, List[dict]], sym_id: str, roles=None) -> List[tuple]:
    recs = symbols.get(sym_id, ())
    if roles is not None:
        recs = [r for r in recs if r["role"] in roles]
    return sorted({_span_key(r) for r in recs})


def build_oracle_cases(manifest: dict) -> List[dict]:
    """Constructs ground-truth verification cases for all 37 implemented LSP methods."""
    syms = manifest["symbols"]
    cases: List[dict] = []

    for fam in manifest["families"]:
        tag = fam["tag"]
        fn_calc = fam["fn_calc"]
        fn_proc = fam["fn_proc"]
        fn_exec = fam["fn_exec"]
        fn_test = fam["fn_test"]
        fn_new = fam["fn_new"]
        fn_scale = fam["fn_scale"]
        fn_compute = fam["fn_compute"]
        fn_boost = fam["fn_boost"]
        struct_rec = fam["struct_rec"]
        struct_alt = fam["struct_alt"]
        trait_base = fam["trait_base"]
        trait_sub = fam["trait_sub"]
        local_item = fam["local_item"]

        calc_def = _first_role(syms, fn_calc, "def")
        calc_calls = [r for r in syms[fn_calc] if r["role"] == "call_ref"]
        rec_def = _first_role(syms, struct_rec, "def")
        rec_tref = _first_role(syms, struct_rec, "type_ref")
        proc_def = _first_role(syms, fn_proc, "def")
        proc_call = _first_role(syms, fn_proc, "call_ref")
        comp_decl = _first_role(syms, fn_compute, "decl")
        comp_call = _first_role(syms, fn_compute, "call_ref")
        local_def = _first_role(syms, local_item, "local_def")
        trait_base_def = _first_role(syms, trait_base, "def")
        trait_sub_def = _first_role(syms, trait_sub, "def")

        # 1. Core 4 ground-truth methods: definition, references, rename, implementation
        for call_rec in calc_calls:
            cases.append({
                "method": "definition",
                "case_id": f"def_{fn_calc}_from_{call_rec['rel_path']}:{call_rec['line']}",
                "cursor": call_rec,
                "expected_spans": [_span_key(calc_def)],
            })
        cases.append({
            "method": "definition",
            "case_id": f"def_{struct_rec}_from_service",
            "cursor": rec_tref,
            "expected_spans": [_span_key(rec_def)],
        })
        cases.append({
            "method": "definition",
            "case_id": f"def_{fn_proc}_from_app",
            "cursor": proc_call,
            "expected_spans": [_span_key(proc_def)],
        })

        for sym_id in (fn_calc, fn_proc, struct_rec):
            cases.append({
                "method": "references",
                "case_id": f"refs_{sym_id}",
                "cursor": _first_role(syms, sym_id, "def"),
                "expected_spans": _all_keys(syms, sym_id),
            })
            cases.append({
                "method": "rename",
                "case_id": f"rename_{sym_id}",
                "cursor": _first_role(syms, sym_id, "def"),
                "new_name": f"{sym_id}_renamed",
                "expected_spans": _all_keys(syms, sym_id),
            })
        for sym_id, role in ((fn_compute, "decl"), (fam["field_val"], "def"), (local_item, "local_def")):
            cases.append({
                "method": "rename",
                "case_id": f"rename_{sym_id}",
                "cursor": _first_role(syms, sym_id, role),
                "new_name": f"{sym_id}_renamed",
                "expected_spans": _all_keys(syms, sym_id),
            })

        cases.append({
            "method": "implementation",
            "case_id": f"impl_trait_{trait_base}",
            "cursor": trait_base_def,
            "expected_spans": sorted([
                _span_key(_first_role(syms, struct_rec, "trait_impl_target")),
                _span_key(_first_role(syms, struct_alt, "trait_impl_target")),
            ]),
        })
        cases.append({
            "method": "implementation",
            "case_id": f"impl_method_{fn_compute}",
            "cursor": comp_decl,
            "expected_spans": _all_keys(syms, fn_compute, roles=("impl_def", "impl_def_alt")),
        })
        cases.append({
            "method": "implementation",
            "case_id": f"impl_struct_{struct_rec}",
            "cursor": rec_def,
            "expected_spans": sorted(
                _all_keys(syms, struct_rec, roles=("inherent_impl", "trait_impl_target", "subtrait_impl_target"))
                + [(rec_def["rel_path"], rec_def["line"] - 1, 0, rec_def["line"] - 1, 0)]
            ),
        })

        # 2. Extended navigation & hierarchy methods
        cases.append({
            "method": "declaration",
            "case_id": f"decl_{fn_compute}",
            "cursor": comp_call,
            "expected_spans": [_span_key(comp_decl)],
        })
        cases.append({
            "method": "type_definition",
            "case_id": f"typedef_{local_item}",
            "cursor": local_def,
            "expected_spans": [_span_key(rec_def)],
        })
        cases.append({
            "method": "document_highlight",
            "case_id": f"dochighlight_{local_item}",
            "cursor": local_def,
            "expected_spans": _all_keys(syms, local_item),
        })
        cases.append({
            "method": "prepare_rename",
            "case_id": f"prep_rename_{fn_calc}",
            "cursor": calc_def,
            "expected_spans": [_span_key(calc_def)],
            "expected_text": fn_calc,
        })
        cases.append({
            "method": "linked_editing_range",
            "case_id": f"linked_edit_{local_item}",
            "cursor": local_def,
            "expected_spans": _all_keys(syms, local_item),
        })

        # Call hierarchy
        cases.append({
            "method": "prepare_call_hierarchy",
            "case_id": f"prep_ch_{fn_calc}",
            "cursor": calc_def,
            "expected_name": fn_calc,
        })
        cases.append({
            "method": "incoming_calls",
            "case_id": f"incoming_{fn_calc}",
            "cursor": calc_def,
            "expected_names": sorted([fn_proc, fn_exec]),
        })
        cases.append({
            "method": "outgoing_calls",
            "case_id": f"outgoing_{fn_proc}",
            "cursor": proc_def,
            "expected_names": sorted([fn_new, fn_calc, fn_boost, fn_compute]),
        })

        # Type hierarchy
        cases.append({
            "method": "prepare_type_hierarchy",
            "case_id": f"prep_th_{trait_sub}",
            "cursor": trait_sub_def,
            "expected_name": trait_sub,
        })
        cases.append({
            "method": "supertypes",
            "case_id": f"supertypes_{struct_rec}",
            "cursor": rec_def,
            "expected_names": sorted([trait_base, trait_sub]),
        })
        cases.append({
            "method": "subtypes",
            "case_id": f"subtypes_{trait_base}",
            "cursor": trait_base_def,
            "expected_names": sorted([struct_rec, struct_alt]),
        })

        # Metadata, symbols, tokens, editing & chained resolves
        cases.append({
            "method": "hover",
            "case_id": f"hover_{fn_calc}",
            "cursor": calc_calls[0],
            "expected_substr": fn_calc,
        })
        cases.append({
            "method": "signature_help",
            "case_id": f"sig_{fn_calc}",
            "cursor": _first_role(syms, f"sig_arg2_{tag}", "sig_arg2"),
            "expected_substr": fn_calc,
            "expected_active_param": 1,
        })
        cases.append({
            "method": "inlay_hint",
            "case_id": f"inlay_{fn_proc}",
            "cursor": local_def,
            "expected_substr": struct_rec,
        })
        cases.append({
            "method": "inlay_hint_resolve",
            "case_id": f"inlay_resolve_{fn_proc}",
            "cursor": local_def,
            "expected_substr": struct_rec,
        })
        cases.append({
            "method": "document_symbol",
            "case_id": f"docsym_models_{tag}",
            "cursor": calc_def,
            "expected_names": sorted([trait_base, trait_sub, struct_rec, struct_alt, fn_calc, fn_new, fn_scale]),
        })
        cases.append({
            "method": "workspace_symbol",
            "case_id": f"wssym_{fn_calc}",
            "cursor": calc_def,
            "query": fn_calc,
            "expected_name": fn_calc,
        })
        cases.append({
            "method": "workspace_symbol_resolve",
            "case_id": f"wssym_resolve_{fn_calc}",
            "cursor": calc_def,
            "query": fn_calc,
            "expected_name": fn_calc,
        })
        cases.append({
            "method": "folding_range",
            "case_id": f"fold_models_{tag}",
            "cursor": calc_def,
            "expected_folds": [
                (f["start_line"], f["end_line"])
                for f in manifest["files"]["crates/core_pkg/src/models.rs"]["folds"]
            ],
        })
        cases.append({
            "method": "selection_range",
            "case_id": f"selrange_{fn_calc}",
            "cursor": calc_calls[0],
            "expected_inner": _span_key(calc_calls[0]),
        })
        doc_rec = _first_role(syms, f"doc_{tag}", "doc_link")
        cases.append({
            "method": "document_link",
            "case_id": f"doclink_{tag}",
            "cursor": doc_rec,
            "expected_url": fam["doc_url"],
        })
        cases.append({
            "method": "document_link_resolve",
            "case_id": f"doclink_resolve_{tag}",
            "cursor": doc_rec,
            "expected_url": fam["doc_url"],
        })
        for st_method in ("semantic_tokens_full", "semantic_tokens_delta", "semantic_tokens_range"):
            cases.append({
                "method": st_method,
                "case_id": f"{st_method}_{fn_proc}",
                "cursor": proc_def,
                "expected_token": (proc_def["line"], proc_def["col"], proc_def["end_col"] - proc_def["col"]),
            })
        scale_call = _first_role(syms, fn_scale, "call_ref")
        cases.append({
            "method": "completion",
            "case_id": f"comp_{fn_scale}",
            "cursor": scale_call,
            "expected_name": fn_scale,
        })
        cases.append({
            "method": "completion_resolve",
            "case_id": f"comp_resolve_{fn_scale}",
            "cursor": scale_call,
            "expected_name": fn_scale,
        })
        cases.append({
            "method": "code_action",
            "case_id": f"code_action_{fn_proc}",
            "cursor": proc_call,
        })
        cases.append({
            "method": "code_action_resolve",
            "case_id": f"code_action_resolve_{fn_proc}",
            "cursor": proc_call,
        })
        messy_def = _first_role(syms, f"messy_{tag}", "def")
        for fmt_method in ("formatting", "range_formatting", "on_type_formatting"):
            cases.append({
                "method": fmt_method,
                "case_id": f"{fmt_method}_{tag}",
                "cursor": messy_def,
                "expected_substr": f"pub fn messy_{tag}(x: i64, y: i64) -> i64",
            })
        cases.append({
            "method": "document_diagnostic",
            "case_id": f"doc_diag_{fn_proc}",
            "cursor": proc_def,
        })
        cases.append({
            "method": "workspace_diagnostic",
            "case_id": f"ws_diag_{tag}",
            "cursor": proc_def,
        })
        exec_def = _first_role(syms, fn_exec, "def")
        test_def = _first_role(syms, fn_test, "def")
        cases.append({
            "method": "code_lens",
            "case_id": f"codelens_{tag}",
            "cursor": exec_def,
            "expected_lines": sorted([exec_def["line"], test_def["line"]]),
        })
        cases.append({
            "method": "code_lens_resolve",
            "case_id": f"codelens_resolve_{tag}",
            "cursor": exec_def,
        })
        cases.append({
            "method": "moniker",
            "case_id": f"moniker_{fn_calc}",
            "cursor": calc_def,
            "expected_substr": fn_calc,
        })

    return cases
