#!/usr/bin/env python3
#
# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
#

"""Generates test coverage matrix (docs/TEST_COVERAGE.md) for RIDX across 53 LSP rows."""

import os
import re
import sys
from pathlib import Path

TARGET_PER_METHOD = 50

DOMAINS = [
    (
        "Domain 1: Lifecycle & Control",
        [
            ("initialize / initialized", ["initialize_", "initialized_"]),
            ("shutdown", ["shutdown_"]),
            ("exit", ["exit_"]),
            ("$/cancelRequest", ["cancel_request_", "cancelrequest_"]),
            ("$/setTrace", ["set_trace_"]),
        ],
    ),
    (
        "Domain 2: Diagnostics & Flycheck",
        [
            ("textDocument/diagnostic", ["pull_diagnostic_", "pull_client_", "document_diagnostic_", "diagnostic_"]),
            ("workspace/diagnostic", ["workspace_diagnostic_", "workspace_diagnostic_report", "workspace_diagnostics_"]),
            ("publishDiagnostics / flycheck", [
                "publish_diagnostics_", "publish_diagnostic_", "flycheck_",
                "syntax_", "publish_syntax_diagnostics_", "reconcile_diagnostics_",
                "diagnostics_ignores_comment_example",
                "parse_compiler_", "parse_real_", "parse_lint_", "parse_error_",
                "parse_macro_", "parse_missing_", "parse_multiline_", "parse_no_primary_",
                "parse_related_", "parse_simple_", "parse_utf16_", "parse_children_",
            ]),
        ],
    ),
    (
        "Domain 3: Synchronization & Coordinates",
        [
            ("textDocument/didOpen", ["did_open_"]),
            ("textDocument/didChange", ["did_change_"]),
            ("textDocument/didSave", ["did_save_", "case_9_line_shift_after_did_save_without_reindex"]),
            ("textDocument/didClose", ["did_close_"]),
            ("willSave / willSaveWaitUntil", ["will_save_wait_until_", "will_save_"]),
        ],
    ),
    (
        "Domain 4: Actions & Refactoring",
        [
            ("textDocument/codeAction", [
                "code_action_", "codeaction_", "extract_variable_", "organize_imports_",
                "domain4_pre_initialize_requests_rejected_with_32002",
                "stream_code_action_did_change_resolve_conflict",
            ]),
            ("codeAction/resolve", ["code_action_resolve_", "real_l2_external_dependency_rejection"]),
        ],
    ),
    (
        "Domain 5: Intelligence & Completion",
        [
            ("textDocument/completion", [
                "completion_", "rich_type_completion_", "domain5_pre_initialize_requests_rejected_with_32002",
                "domain5_e2e_full_intelligence_stream", "domain5_concurrent_intelligence_requests_stress",
                "generic_param_completion_in_fn_and_impl_body", "out_of_bounds_line_completion_returns_empty",
                "inline_completion_unicode_edit",
            ]),
            ("completionItem/resolve", ["completion_item_resolve_", "completion_resolve_"]),
            ("textDocument/hover", ["hover_"]),
            ("textDocument/signatureHelp", ["signature_help_"]),
            ("textDocument/inlayHint / resolve", ["inlay_hint_resolve_", "inlay_hint_"]),
            ("textDocument/inlineValue", ["inline_value_"]),
            ("textDocument/moniker", ["moniker_resolve_", "moniker_"]),
        ],
    ),
    (
        "Domain 6: Hierarchies",
        [
            ("prepareCallHierarchy", ["prepare_call_hierarchy_"]),
            ("callHierarchy/incomingCalls", [
                "call_hierarchy_incoming_calls_", "incoming_calls_", "call_hierarchy_prepare_and_incoming",
            ]),
            ("callHierarchy/outgoingCalls", [
                "call_hierarchy_outgoing_calls_", "outgoing_calls_", "call_hierarchy_outgoing",
            ]),
            ("prepareTypeHierarchy", ["prepare_type_hierarchy_"]),
            ("typeHierarchy/supertypes", [
                "type_hierarchy_supertypes_", "supertypes_", "type_hierarchy_supertypes_and_subtypes",
            ]),
            ("typeHierarchy/subtypes", [
                "type_hierarchy_subtypes_", "subtypes_",
                "type_hierarchy_cross_crate_impl", "type_hierarchy_derive_impl", "type_hierarchy_robot_greeter",
            ]),
        ],
    ),
    (
        "Domain 7: Presentation",
        [
            ("textDocument/semanticTokens/full / delta / range", ["semantic_tokens_"]),
            ("textDocument/foldingRange", ["folding_range_", "folding_"]),
            ("textDocument/selectionRange", ["selection_range_"]),
            ("textDocument/documentLink / resolve", ["document_link_resolve_", "document_link_"]),
            ("textDocument/documentColor / colorPresentation", ["color_presentation_", "document_color_"]),
            ("textDocument/codeLens / resolve", ["code_lens_resolve_", "code_lens_"]),
        ],
    ),
    (
        "Domain 8: Editing & Workspace Edits",
        [
            ("textDocument/prepareRename", ["prepare_rename_"]),
            ("textDocument/rename", [
                "rename_", "cross_file_workspace_rename_execution", "case_8_external_dependency_rename_safety",
                "case_10_stale_rename_overlay_resolution", "ntm_overlay_mutations_and_editing_pipeline",
            ]),
            ("textDocument/linkedEditingRange", ["linked_editing_range_", "linked_editing_"]),
            ("textDocument/formatting / rangeFormatting", ["range_formatting_", "formatting_"]),
            ("textDocument/onTypeFormatting", ["on_type_formatting_", "ontype_formatting_"]),
            ("workspace/executeCommand", [
                "execute_command_", "execute_commands_", "workspace_execute_commands",
                "stream_execute_command",
                "apply_edit", "apply_workspace_edit", "workspace_edit",
                "outbound_apply_edit", "partial_workspace_edit",
                "sequential_workspace_edit",
                "open_document_command", "client_ack_does_not_rewrite",
                "client_advancing_to_version_2_rejects_version_1_apply_edit",
                "combined_versions_retain_initial_client_snapshot",
                "multiple_arguments_preserve_all_edits",
                "same_position_inserts_preserve_declared_order",
                "two_concurrent_pending_edits",
            ]),
        ],
    ),
    (
        "Domain 9: Navigation & Symbols",
        [
            ("textDocument/definition / declaration (delegating)", [
                "definition_", "declaration_", "goto_def",
                "reexport_item_definition_and_declaration", "traits_call_generic_method_definition",
                "sysroot_atomic_load_definition", "goto_definition_inside_cfg_test_struct",
                "goto_definition_inside_cfg_test_method", "reexport_definition_resolution",
                "staleness_definition_in_modified_hunk_abstains", "case_9_line_shift_definition_and_references",
                "declaration_definition_parity_contract", "declaration_matches_definition_same_file",
                "case_1_associated_type_navigation", "case_2_trait_method_concrete_vs_generic",
                "case_3_two_versions_of_same_crate", "case_4_exact_ident_span_and_raw_idents",
                "case_5_macros_and_derives_navigation", "case_6_field_shorthand_and_destructuring",
                "case_7_aliased_use_and_reexports", "case_cross_crate_ident_name_isolation",
                "case_sysroot_atomic_tyalias_navigation_and_refs",
                "macro_included_file_navigation", "macro_multiplier_navigation_and_refs",
                "multiroot_cross_crate_navigation", "reexport_aliased_type_navigation",
                "reexport_statement_navigation", "struct_field_shorthand_navigation_and_refs",
                "sysroot_ops_range_navigation", "overlay_preserves_navigation",
                "ntm_end_to_end_lsp_pipeline", "ntm_hierarchy_and_protocol_invariants",
            ]),
            ("textDocument/typeDefinition", ["type_definition_"]),
            ("textDocument/implementation", ["implementation_"]),
            ("textDocument/references", [
                "references_", "raw_ident_navigation_and_references",
                "cross_crate_dispatch_and_references", "traits_default_method_references",
                "cross_crate_reexport_references_and_rename",
            ]),
            ("textDocument/documentHighlight", ["document_highlight_"]),
            ("textDocument/documentSymbol", ["document_symbol_", "case_11_new_symbol_in_live_buffer"]),
            ("workspace/symbol", ["workspace_symbol_", "multiroot_symbols_search_across_crates"]),
            ("workspaceSymbol/resolve", ["workspace_symbol_resolve_"]),
        ],
    ),
    (
        "Domain 10: Multi-root & File Events",
        [
            ("workspace/didChangeWorkspaceFolders", [
                "did_change_workspace_folders_", "workspace_folders_", "multiroot_dynamic_workspace_folders",
            ]),
            ("workspace/didChangeWatchedFiles", ["did_change_watched_files_", "watched_files_"]),
            ("workspace/didCreateFiles / willCreateFiles", ["will_create_files_", "did_create_files_"]),
            ("workspace/didRenameFiles / willRenameFiles", ["will_rename_files_", "did_rename_files_"]),
            ("workspace/didDeleteFiles / willDeleteFiles", ["will_delete_files_", "did_delete_files_"]),
        ],
    ),
]

TIER_PREFIXES = [
    "test_l1_", "test_l2_", "test_e2e_",
    "test_domain10_", "test_domain9_", "test_domain8_", "test_domain7_", "test_domain6_",
    "test_domain5_", "test_domain4_", "test_domain3_", "test_domain2_", "test_domain1_",
    "test_refactor_", "test_hermetic_", "test_review_v4651_",
    "test_",
]


def strip_tier_prefixes(name: str) -> str:
    changed = True
    while changed:
        changed = False
        for pfx in TIER_PREFIXES:
            if name.startswith(pfx):
                name = name[len(pfx):]
                changed = True
                break
    return name


def build_sorted_rules():
    rules = []
    for _, methods in DOMAINS:
        for method, prefixes in methods:
            for prefix in prefixes:
                rules.append((method, prefix))
    rules.sort(key=lambda r: len(r[1]), reverse=True)
    return rules


def extract_tests_from_file(file_path):
    with open(file_path, "r", encoding="utf-8") as f:
        return re.findall(r"#\[test\]\s*(?:#\[[^\]]+\]\s*)*fn\s+([a-zA-Z0-9_]+)\s*\(", f.read())


def classify_tests(test_names, sorted_rules):
    classified = {method: [] for _, methods in DOMAINS for method, _ in methods}
    unclassified = []
    for tname in test_names:
        norm = strip_tier_prefixes(tname)
        matched = False
        for method, prefix in sorted_rules:
            if norm.startswith(prefix) or prefix == norm or (prefix.endswith("_") and prefix[:-1] == norm):
                classified[method].append(tname)
                matched = True
                break
            if tname.startswith(prefix) or prefix == tname or (prefix.endswith("_") and prefix[:-1] == tname):
                classified[method].append(tname)
                matched = True
                break
        if not matched:
            unclassified.append(tname)
    return classified, sorted(unclassified)


def main():
    repo_root = Path(__file__).resolve().parent.parent
    e2e_dir = repo_root / "crates" / "ridx" / "tests" / "e2e"
    src_dir = repo_root / "crates" / "ridx" / "src"

    main_rs = e2e_dir / "main.rs"
    active_e2e_files = set()
    if main_rs.is_file():
        with open(main_rs, "r", encoding="utf-8") as f:
            for line in f:
                m = re.match(r"\s*mod\s+([a-zA-Z0-9_]+)\s*;", line)
                if m:
                    cand = e2e_dir / f"{m.group(1)}.rs"
                    if cand.is_file():
                        active_e2e_files.add(cand)

    e2e_tests = []
    for p in sorted(active_e2e_files):
        e2e_tests.extend(extract_tests_from_file(p))

    src_tests = []
    for p in sorted(src_dir.rglob("*.rs")):
        src_tests.extend(extract_tests_from_file(p))

    sorted_rules = build_sorted_rules()
    e2e_classified, unclassified = classify_tests(e2e_tests, sorted_rules)
    src_classified, _ = classify_tests(src_tests, sorted_rules)

    if "--unclassified" in sys.argv:
        print(f"Total unclassified tests: {len(unclassified)}")
        for t in unclassified:
            print(f"  {t}")
        return

    total_e2e, total_src = len(e2e_tests), len(src_tests)
    all_methods = [method for _, methods in DOMAINS for method, _ in methods]
    rows_ge_20 = sum(1 for m in all_methods if len(e2e_classified[m]) >= 20)
    rows_ge_50 = sum(1 for m in all_methods if len(e2e_classified[m]) >= 50)
    num_rows = len(all_methods)

    out = [
        f"# RIDX Test Coverage Matrix across {num_rows} LSP Methods", "",
        f"> **Note on Method Count ({num_rows} methods)**: The matrix covers {num_rows} distinct LSP capability rows across 10 domains. LSP `textDocument/declaration` is delegated directly to `textDocument/definition` via `handle_declaration` and tracked jointly in `textDocument/definition / declaration (delegating)`.",
        "> **Note on Src Tests**: The `Src Tests` column is not yet populated by the classifier (rules currently target E2E test suites; unit test classification is deferred to a future wave).", "",
        f"- **Total E2E Tests**: {total_e2e}",
        f"- **Total Unit/Src Tests**: {total_src}",
        f"- **Classified E2E Tests**: {total_e2e - len(unclassified)}",
        f"- **Unclassified E2E Tests**: {len(unclassified)}",
        f"- **Rows >= 20 E2E**: {rows_ge_20} / {num_rows}",
        f"- **Rows >= 50 E2E**: {rows_ge_50} / {num_rows}", "",
        "| Domain | Method / Capability | E2E Tests | Src Tests | Total | Status |",
        "| :--- | :--- | :---: | :---: | :---: | :---: |",
    ]

    for domain_name, methods in DOMAINS:
        for method, _ in methods:
            e_cnt = len(e2e_classified[method])
            s_cnt = len(src_classified[method])
            status = "✅ Complete" if e_cnt >= 50 else "🟡 Wave 1 Pass (>= 20)" if e_cnt >= 20 else f"🔴 In Progress ({e_cnt}/50)" if e_cnt > 0 else "⚪ Zero E2E"
            out.append(f"| {domain_name} | `{method}` | {e_cnt} | {s_cnt} | {e_cnt + s_cnt} | {status} |")

    out.extend(["", "## Method-level Test Catalog", ""])
    for domain_name, methods in DOMAINS:
        out.append(f"### {domain_name}\n")
        for method, _ in methods:
            tests = sorted(e2e_classified[method])
            out.append(f"#### `{method}` ({len(tests)} e2e tests)")
            out.extend([f"- `{t}`" for t in tests] if tests else ["_No classified e2e tests yet._"])
            out.append("")

    out.extend([
        "### Unclassified E2E Tests", "",
        f"Total unclassified tests: {len(unclassified)} (multi-purpose, property, or helper harness tests)", "",
    ] + [f"- `{t}`" for t in unclassified] + [
        "", "---",
        f"coverage: e2e {total_e2e} total, {rows_ge_20} rows >= 20, {rows_ge_50} rows >= 50 of {num_rows}", "",
    ])

    target_file = repo_root / "docs" / "TEST_COVERAGE.md"
    target_file.parent.mkdir(parents=True, exist_ok=True)
    with open(target_file, "w", encoding="utf-8") as f:
        f.write("\n".join(out))

    print(f"Generated {target_file}")
    print(f"coverage: e2e {total_e2e} total, {rows_ge_20} rows >= 20, {rows_ge_50} rows >= 50 of {num_rows}")


if __name__ == "__main__":
    main()
