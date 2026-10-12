# RIDX Test Coverage Matrix across 53 LSP Methods

> **Note on Method Count (53 methods)**: The matrix covers 53 distinct LSP capability rows across 10 domains. LSP `textDocument/declaration` is delegated directly to `textDocument/definition` via `handle_declaration` and tracked jointly in `textDocument/definition / declaration (delegating)`.
> **Note on Stubs**: LSP `textDocument/documentColor / colorPresentation (stub)` handler returns `[]` unconditionally; tests pin the stub.
> **Note on Src Tests**: The `Src Tests` column is not yet populated by the classifier (rules currently target E2E test suites; unit test classification is deferred to a future wave).

- **Total E2E Tests**: 706
- **Total Unit/Src Tests**: 410
- **Classified E2E Tests**: 624
- **Unclassified E2E Tests**: 82
- **Rows >= 20 E2E**: 20 / 53
- **Rows >= 50 E2E**: 1 / 53

| Domain | Method / Capability | E2E Tests | Src Tests | Total | Status |
| :--- | :--- | :---: | :---: | :---: | :---: |
| Domain 1: Lifecycle & Control | `initialize / initialized` | 0 | 0 | 0 | ⚪ Zero E2E |
| Domain 1: Lifecycle & Control | `shutdown` | 0 | 0 | 0 | ⚪ Zero E2E |
| Domain 1: Lifecycle & Control | `exit` | 0 | 0 | 0 | ⚪ Zero E2E |
| Domain 1: Lifecycle & Control | `$/cancelRequest` | 0 | 0 | 0 | ⚪ Zero E2E |
| Domain 1: Lifecycle & Control | `$/setTrace` | 0 | 0 | 0 | ⚪ Zero E2E |
| Domain 2: Diagnostics & Flycheck | `textDocument/diagnostic` | 2 | 0 | 2 | 🔴 In Progress (2/50) |
| Domain 2: Diagnostics & Flycheck | `workspace/diagnostic` | 2 | 1 | 3 | 🔴 In Progress (2/50) |
| Domain 2: Diagnostics & Flycheck | `publishDiagnostics / flycheck` | 62 | 0 | 62 | ✅ Complete |
| Domain 3: Synchronization & Coordinates | `textDocument/didOpen` | 9 | 0 | 9 | 🔴 In Progress (9/50) |
| Domain 3: Synchronization & Coordinates | `textDocument/didChange` | 11 | 0 | 11 | 🔴 In Progress (11/50) |
| Domain 3: Synchronization & Coordinates | `textDocument/didSave` | 3 | 0 | 3 | 🔴 In Progress (3/50) |
| Domain 3: Synchronization & Coordinates | `textDocument/didClose` | 3 | 0 | 3 | 🔴 In Progress (3/50) |
| Domain 3: Synchronization & Coordinates | `willSave / willSaveWaitUntil` | 0 | 0 | 0 | ⚪ Zero E2E |
| Domain 4: Actions & Refactoring | `textDocument/codeAction` | 7 | 12 | 19 | 🔴 In Progress (7/50) |
| Domain 4: Actions & Refactoring | `codeAction/resolve` | 1 | 13 | 14 | 🔴 In Progress (1/50) |
| Domain 5: Intelligence & Completion | `textDocument/completion` | 10 | 25 | 35 | 🔴 In Progress (10/50) |
| Domain 5: Intelligence & Completion | `completionItem/resolve` | 0 | 9 | 9 | ⚪ Zero E2E |
| Domain 5: Intelligence & Completion | `textDocument/hover` | 5 | 16 | 21 | 🔴 In Progress (5/50) |
| Domain 5: Intelligence & Completion | `textDocument/signatureHelp` | 1 | 3 | 4 | 🔴 In Progress (1/50) |
| Domain 5: Intelligence & Completion | `textDocument/inlayHint / resolve` | 1 | 9 | 10 | 🔴 In Progress (1/50) |
| Domain 5: Intelligence & Completion | `textDocument/inlineValue` | 0 | 0 | 0 | ⚪ Zero E2E |
| Domain 5: Intelligence & Completion | `textDocument/moniker` | 0 | 0 | 0 | ⚪ Zero E2E |
| Domain 6: Hierarchies | `prepareCallHierarchy` | 20 | 0 | 20 | 🟡 Wave 1 Pass (>= 20) |
| Domain 6: Hierarchies | `callHierarchy/incomingCalls` | 21 | 0 | 21 | 🟡 Wave 1 Pass (>= 20) |
| Domain 6: Hierarchies | `callHierarchy/outgoingCalls` | 22 | 0 | 22 | 🟡 Wave 1 Pass (>= 20) |
| Domain 6: Hierarchies | `prepareTypeHierarchy` | 20 | 0 | 20 | 🟡 Wave 1 Pass (>= 20) |
| Domain 6: Hierarchies | `typeHierarchy/supertypes` | 1 | 0 | 1 | 🔴 In Progress (1/50) |
| Domain 6: Hierarchies | `typeHierarchy/subtypes` | 3 | 0 | 3 | 🔴 In Progress (3/50) |
| Domain 7: Presentation | `textDocument/semanticTokens/full / delta / range` | 23 | 0 | 23 | 🟡 Wave 1 Pass (>= 20) |
| Domain 7: Presentation | `textDocument/foldingRange` | 21 | 0 | 21 | 🟡 Wave 1 Pass (>= 20) |
| Domain 7: Presentation | `textDocument/selectionRange` | 20 | 0 | 20 | 🟡 Wave 1 Pass (>= 20) |
| Domain 7: Presentation | `textDocument/documentLink / resolve` | 20 | 0 | 20 | 🟡 Wave 1 Pass (>= 20) |
| Domain 7: Presentation | `textDocument/documentColor / colorPresentation (stub)` | 20 | 0 | 20 | 🟡 Wave 1 Pass (>= 20) |
| Domain 7: Presentation | `textDocument/codeLens / resolve` | 21 | 0 | 21 | 🟡 Wave 1 Pass (>= 20) |
| Domain 8: Editing & Workspace Edits | `textDocument/prepareRename` | 0 | 10 | 10 | ⚪ Zero E2E |
| Domain 8: Editing & Workspace Edits | `textDocument/rename` | 5 | 17 | 22 | 🔴 In Progress (5/50) |
| Domain 8: Editing & Workspace Edits | `textDocument/linkedEditingRange` | 1 | 7 | 8 | 🔴 In Progress (1/50) |
| Domain 8: Editing & Workspace Edits | `textDocument/formatting / rangeFormatting` | 3 | 8 | 11 | 🔴 In Progress (3/50) |
| Domain 8: Editing & Workspace Edits | `textDocument/onTypeFormatting` | 1 | 5 | 6 | 🔴 In Progress (1/50) |
| Domain 8: Editing & Workspace Edits | `workspace/executeCommand` | 14 | 0 | 14 | 🔴 In Progress (14/50) |
| Domain 9: Navigation & Symbols | `textDocument/definition / declaration (delegating)` | 44 | 7 | 51 | 🟡 Wave 1 Pass (>= 20) |
| Domain 9: Navigation & Symbols | `textDocument/typeDefinition` | 20 | 0 | 20 | 🟡 Wave 1 Pass (>= 20) |
| Domain 9: Navigation & Symbols | `textDocument/implementation` | 16 | 0 | 16 | 🔴 In Progress (16/50) |
| Domain 9: Navigation & Symbols | `textDocument/references` | 20 | 1 | 21 | 🟡 Wave 1 Pass (>= 20) |
| Domain 9: Navigation & Symbols | `textDocument/documentHighlight` | 19 | 0 | 19 | 🔴 In Progress (19/50) |
| Domain 9: Navigation & Symbols | `textDocument/documentSymbol` | 16 | 0 | 16 | 🔴 In Progress (16/50) |
| Domain 9: Navigation & Symbols | `workspace/symbol` | 20 | 0 | 20 | 🟡 Wave 1 Pass (>= 20) |
| Domain 9: Navigation & Symbols | `workspaceSymbol/resolve` | 15 | 8 | 23 | 🔴 In Progress (15/50) |
| Domain 10: Multi-root & File Events | `workspace/didChangeWorkspaceFolders` | 21 | 0 | 21 | 🟡 Wave 1 Pass (>= 20) |
| Domain 10: Multi-root & File Events | `workspace/didChangeWatchedFiles` | 20 | 0 | 20 | 🟡 Wave 1 Pass (>= 20) |
| Domain 10: Multi-root & File Events | `workspace/didCreateFiles / willCreateFiles` | 20 | 0 | 20 | 🟡 Wave 1 Pass (>= 20) |
| Domain 10: Multi-root & File Events | `workspace/didRenameFiles / willRenameFiles` | 20 | 0 | 20 | 🟡 Wave 1 Pass (>= 20) |
| Domain 10: Multi-root & File Events | `workspace/didDeleteFiles / willDeleteFiles` | 20 | 0 | 20 | 🟡 Wave 1 Pass (>= 20) |

## Method-level Test Catalog

### Domain 1: Lifecycle & Control

#### `initialize / initialized` (0 e2e tests)
_No classified e2e tests yet._

#### `shutdown` (0 e2e tests)
_No classified e2e tests yet._

#### `exit` (0 e2e tests)
_No classified e2e tests yet._

#### `$/cancelRequest` (0 e2e tests)
_No classified e2e tests yet._

#### `$/setTrace` (0 e2e tests)
_No classified e2e tests yet._

### Domain 2: Diagnostics & Flycheck

#### `textDocument/diagnostic` (2 e2e tests)
- `test_e2e_pull_client_without_refresh_support_no_server_refresh_request`
- `test_e2e_pull_diagnostic_client_no_publish_on_did_change`

#### `workspace/diagnostic` (2 e2e tests)
- `test_l2_workspace_diagnostic_report`
- `test_workspace_diagnostics_reports_syntax_errors_in_open_overlays`

#### `publishDiagnostics / flycheck` (62 e2e tests)
- `test_diagnostics_ignores_comment_example`
- `test_e2e_flycheck_e0425_line_shifted_after_lines_inserted_above`
- `test_e2e_flycheck_e0425_preserved_after_did_change`
- `test_flycheck_arc_clone_shares_atomic_generation`
- `test_flycheck_cancel_increments_generation`
- `test_flycheck_concurrent_cancellation_thread_safety`
- `test_flycheck_diagnostic_clearing_logic_on_clean_build`
- `test_flycheck_empty_diagnostics_clearing_payload`
- `test_flycheck_generation_mismatch_drops_stale_notification`
- `test_flycheck_generation_survives_rapid_interleaved_resets`
- `test_flycheck_multiple_cancels_strictly_monotonic`
- `test_flycheck_multiple_errors_same_file_accumulated`
- `test_flycheck_notifier_invocation`
- `test_flycheck_notifier_replacement_safety`
- `test_flycheck_publish_payload_schema`
- `test_flycheck_shutdown_idempotent`
- `test_flycheck_state_cancellation`
- `test_flycheck_state_default_matches_new`
- `test_flycheck_state_initial_generation_zero`
- `test_flycheck_state_latest_diagnostics_cache`
- `test_flycheck_stream_ignores_non_json_and_compiler_artifacts`
- `test_flycheck_stream_multi_file_diagnostics`
- `test_flycheck_stream_single_file_diagnostics`
- `test_parse_children_notes_and_helps`
- `test_parse_compiler_message_error_with_spans`
- `test_parse_compiler_message_includes_children_notes_and_utf16_columns`
- `test_parse_compiler_warning_and_severities`
- `test_parse_error_outside_workspace_root`
- `test_parse_lint_with_suggested_replacement`
- `test_parse_macro_expansion_span_preservation`
- `test_parse_missing_code_field_produces_null_code`
- `test_parse_multiline_span_start_and_end`
- `test_parse_no_primary_span_or_empty_spans_returns_none`
- `test_parse_real_clean_build_artifacts_ignored_fixture`
- `test_parse_real_e0425_unresolved_name_fixture`
- `test_parse_real_macro_expansion_callsite_fixture`
- `test_parse_real_multi_crate_subcrate_fixture`
- `test_parse_real_unused_warning_with_suggestion_fixture`
- `test_parse_related_information_secondary_spans`
- `test_parse_simple_compiler_error`
- `test_parse_utf16_columns_with_astral_emojis`
- `test_parse_utf16_multibyte_greek_and_german`
- `test_publish_syntax_diagnostics_overlay_lookup`
- `test_reconcile_diagnostics_preserves_non_overlapping_errors_on_same_line`
- `test_syntax_and_flycheck_diagnostics_coexistence_and_deduplication`
- `test_syntax_clean_code_zero_diagnostics`
- `test_syntax_comments_ignore_delimiters`
- `test_syntax_delimiters_inside_char_and_string_literals`
- `test_syntax_empty_and_whitespace_only`
- `test_syntax_interleaved_mismatched_delimiters`
- `test_syntax_let_type_mismatch_bool_expected_int_found`
- `test_syntax_let_type_mismatch_int_expected_str_found`
- `test_syntax_let_type_mismatch_str_expected_int_found`
- `test_syntax_let_type_mismatch_usize_bool`
- `test_syntax_nested_block_comments`
- `test_syntax_properly_closed_raw_string_with_internal_quotes`
- `test_syntax_unclosed_block_comment_at_eof`
- `test_syntax_unclosed_brace_at_eof`
- `test_syntax_unclosed_parenthesis_and_bracket`
- `test_syntax_unclosed_raw_string_single_and_multi_hash`
- `test_syntax_unclosed_string_literal`
- `test_syntax_unmatched_closing_delimiters`

### Domain 3: Synchronization & Coordinates

#### `textDocument/didOpen` (9 e2e tests)
- `test_did_open_already_open_replaces_cleanly`
- `test_did_open_crlf_line_endings_handling`
- `test_did_open_empty_document`
- `test_did_open_fresh_document`
- `test_did_open_invalidates_position_cache`
- `test_did_open_multi_reader_provenance_fast_path`
- `test_did_open_multibyte_unicode_and_emojis`
- `test_did_open_with_provenance_compiled`
- `test_did_open_with_provenance_uncompiled`

#### `textDocument/didChange` (11 e2e tests)
- `test_did_change_full_content_sync`
- `test_did_change_incremental_multiline_delete`
- `test_did_change_incremental_multiline_insert`
- `test_did_change_incremental_single_line_delete`
- `test_did_change_incremental_single_line_insert`
- `test_did_change_multi_change_batch_in_order`
- `test_did_change_reparses_symbols_and_scopes`
- `test_did_change_unopened_document_implicit_open`
- `test_did_change_version_duplicate_or_zero`
- `test_did_change_version_monotonic_increments`
- `test_did_change_version_skip_tolerance`

#### `textDocument/didSave` (3 e2e tests)
- `test_case_9_line_shift_after_did_save_without_reindex`
- `test_did_save_triggers_flycheck`
- `test_did_save_updates_disk_sync_state`

#### `textDocument/didClose` (3 e2e tests)
- `test_did_close_evicts_overlay`
- `test_did_close_invalidates_position_converter`
- `test_did_close_unopened_document_is_noop`

#### `willSave / willSaveWaitUntil` (0 e2e tests)
_No classified e2e tests yet._

### Domain 4: Actions & Refactoring

#### `textDocument/codeAction` (7 e2e tests)
- `test_domain4_stream_code_action_did_change_resolve_conflict`
- `test_extract_variable_inside_closure_rejected`
- `test_extract_variable_multiline_short_circuit_and_loop_rejected`
- `test_extract_variable_unicode_and_dependency_order`
- `test_organize_imports_multiline_items`
- `test_refactor_extract_variable_execution`
- `test_refactor_organize_imports_execution`

#### `codeAction/resolve` (1 e2e tests)
- `test_domain4_real_l2_external_dependency_rejection`

### Domain 5: Intelligence & Completion

#### `textDocument/completion` (10 e2e tests)
- `test_l2_completion_generic_trait_bound`
- `test_l2_completion_latency_benchmark`
- `test_l2_completion_local_mod_shadows_sysroot`
- `test_l2_completion_prelude_types_without_use`
- `test_l2_completion_receiver_isolation`
- `test_l2_completion_sysroot_unshadowed_when_no_local_mod`
- `test_l2_generic_param_completion_in_fn_and_impl_body`
- `test_l2_rich_type_completion_latency_benchmark`
- `test_out_of_bounds_line_completion_returns_empty`
- `test_review_v4651_inline_completion_unicode_edit`

#### `completionItem/resolve` (0 e2e tests)
_No classified e2e tests yet._

#### `textDocument/hover` (5 e2e tests)
- `test_l2_hover_inside_cfg_test_method`
- `test_l2_hover_inside_cfg_test_struct`
- `test_l2_hover_on_assoc_type_extract`
- `test_l2_hover_on_generic_trait_method`
- `test_l2_hover_on_inferred_vec`

#### `textDocument/signatureHelp` (1 e2e tests)
- `test_l2_signature_help_cross_crate_and_trait`

#### `textDocument/inlayHint / resolve` (1 e2e tests)
- `test_l2_inlay_hint_utf16_with_emoji`

#### `textDocument/inlineValue` (0 e2e tests)
_No classified e2e tests yet._

#### `textDocument/moniker` (0 e2e tests)
_No classified e2e tests yet._

### Domain 6: Hierarchies

#### `prepareCallHierarchy` (20 e2e tests)
- `test_prepare_call_hierarchy_associated_function`
- `test_prepare_call_hierarchy_cursor_on_non_function_returns_empty`
- `test_prepare_call_hierarchy_cursor_on_whitespace_returns_empty`
- `test_prepare_call_hierarchy_dispatcher_echoes_numeric_id`
- `test_prepare_call_hierarchy_dispatcher_echoes_string_id`
- `test_prepare_call_hierarchy_empty_file_returns_empty`
- `test_prepare_call_hierarchy_method_with_signature`
- `test_prepare_call_hierarchy_missing_position_param_returns_empty`
- `test_prepare_call_hierarchy_multiple_functions_selection`
- `test_prepare_call_hierarchy_nonexistent_file_returns_empty`
- `test_prepare_call_hierarchy_overlay_did_change_updates_symbol`
- `test_prepare_call_hierarchy_overlay_did_close_falls_back`
- `test_prepare_call_hierarchy_overlay_live_buffer_precedence`
- `test_prepare_call_hierarchy_position_at_fn_name_end`
- `test_prepare_call_hierarchy_position_at_fn_name_start`
- `test_prepare_call_hierarchy_position_beyond_eof_returns_empty`
- `test_prepare_call_hierarchy_range_equals_selection_range`
- `test_prepare_call_hierarchy_sequential_requests_isolation`
- `test_prepare_call_hierarchy_single_free_function`
- `test_prepare_call_hierarchy_symbol_kind_function`

#### `callHierarchy/incomingCalls` (21 e2e tests)
- `test_call_hierarchy_incoming_calls_after_overlay_did_change`
- `test_call_hierarchy_incoming_calls_after_overlay_did_close`
- `test_call_hierarchy_incoming_calls_caller_range_body_end_bounded`
- `test_call_hierarchy_incoming_calls_dispatcher_echoes_numeric_id`
- `test_call_hierarchy_incoming_calls_dispatcher_echoes_string_id`
- `test_call_hierarchy_incoming_calls_empty_file_returns_empty`
- `test_call_hierarchy_incoming_calls_from_item_uri_and_range_correct`
- `test_call_hierarchy_incoming_calls_from_ranges_contain_call_span`
- `test_call_hierarchy_incoming_calls_isolated_successive_requests`
- `test_call_hierarchy_incoming_calls_missing_item_param_returns_empty`
- `test_call_hierarchy_incoming_calls_missing_uri_in_item_returns_empty`
- `test_call_hierarchy_incoming_calls_multiple_callers_in_same_file`
- `test_call_hierarchy_incoming_calls_multiple_calls_from_same_caller`
- `test_call_hierarchy_incoming_calls_non_call_reference_ignored`
- `test_call_hierarchy_incoming_calls_nonexistent_item_file_returns_empty`
- `test_call_hierarchy_incoming_calls_overlay_text_call_site_detection`
- `test_call_hierarchy_incoming_calls_roundtrip_from_prepare`
- `test_call_hierarchy_incoming_calls_single_caller_in_same_file`
- `test_call_hierarchy_incoming_calls_unknown_item_returns_empty`
- `test_call_hierarchy_incoming_calls_zero_callers_returns_empty`
- `test_l2_call_hierarchy_prepare_and_incoming`

#### `callHierarchy/outgoingCalls` (22 e2e tests)
- `test_call_hierarchy_outgoing_calls_declaration_fn_filtered`
- `test_call_hierarchy_outgoing_calls_dispatcher_echoes_numeric_id`
- `test_call_hierarchy_outgoing_calls_dispatcher_echoes_string_id`
- `test_call_hierarchy_outgoing_calls_empty_file_returns_empty`
- `test_call_hierarchy_outgoing_calls_generic_turbofish_syntax`
- `test_call_hierarchy_outgoing_calls_isolated_successive_requests`
- `test_call_hierarchy_outgoing_calls_keywords_filtered`
- `test_call_hierarchy_outgoing_calls_missing_item_param_returns_empty`
- `test_call_hierarchy_outgoing_calls_missing_uri_in_item_returns_empty`
- `test_call_hierarchy_outgoing_calls_multiline_call_site`
- `test_call_hierarchy_outgoing_calls_multiple_callees`
- `test_call_hierarchy_outgoing_calls_multiple_calls_same_callee`
- `test_call_hierarchy_outgoing_calls_overlay_did_change`
- `test_call_hierarchy_outgoing_calls_overlay_did_close_falls_back`
- `test_call_hierarchy_outgoing_calls_overlay_text_precedence`
- `test_call_hierarchy_outgoing_calls_recursive_call_self_ignored`
- `test_call_hierarchy_outgoing_calls_roundtrip_from_prepare`
- `test_call_hierarchy_outgoing_calls_single_callee`
- `test_call_hierarchy_outgoing_calls_unknown_callee_ignored`
- `test_call_hierarchy_outgoing_calls_zero_callees_returns_empty`
- `test_l2_call_hierarchy_outgoing`
- `test_l2_call_hierarchy_outgoing_with_array_and_braces`

#### `prepareTypeHierarchy` (20 e2e tests)
- `test_prepare_type_hierarchy_cursor_on_function_returns_empty`
- `test_prepare_type_hierarchy_cursor_on_whitespace_returns_empty`
- `test_prepare_type_hierarchy_dispatcher_echoes_numeric_id`
- `test_prepare_type_hierarchy_dispatcher_echoes_string_id`
- `test_prepare_type_hierarchy_empty_file_returns_empty`
- `test_prepare_type_hierarchy_enum_definition`
- `test_prepare_type_hierarchy_missing_position_param_returns_empty`
- `test_prepare_type_hierarchy_multiple_types_selection`
- `test_prepare_type_hierarchy_nonexistent_file_returns_empty`
- `test_prepare_type_hierarchy_overlay_did_change_updates_symbol`
- `test_prepare_type_hierarchy_overlay_did_close_falls_back`
- `test_prepare_type_hierarchy_overlay_live_buffer_precedence`
- `test_prepare_type_hierarchy_position_at_type_name_end`
- `test_prepare_type_hierarchy_position_at_type_name_start`
- `test_prepare_type_hierarchy_position_beyond_eof_returns_empty`
- `test_prepare_type_hierarchy_range_equals_selection_range`
- `test_prepare_type_hierarchy_sequential_requests_isolation`
- `test_prepare_type_hierarchy_struct_definition`
- `test_prepare_type_hierarchy_trait_definition`
- `test_prepare_type_hierarchy_type_alias_definition`

#### `typeHierarchy/supertypes` (1 e2e tests)
- `test_l2_type_hierarchy_supertypes_and_subtypes`

#### `typeHierarchy/subtypes` (3 e2e tests)
- `test_l2_type_hierarchy_cross_crate_impl`
- `test_l2_type_hierarchy_derive_impl`
- `test_l2_type_hierarchy_robot_greeter`

### Domain 7: Presentation

#### `textDocument/semanticTokens/full / delta / range` (23 e2e tests)
- `test_l2_semantic_tokens_delta_caching_and_edits`
- `test_l2_semantic_tokens_full_compiler_grounded`
- `test_l2_semantic_tokens_range_bounded`
- `test_semantic_tokens_delta_echoes_string_id`
- `test_semantic_tokens_delta_identical_data_returns_empty_edits`
- `test_semantic_tokens_delta_modified_data_returns_edit`
- `test_semantic_tokens_delta_uncached_id_returns_full_data`
- `test_semantic_tokens_full_dispatcher_echoes_numeric_id`
- `test_semantic_tokens_full_dispatcher_echoes_string_id`
- `test_semantic_tokens_full_empty_file_returns_empty_data`
- `test_semantic_tokens_full_multiple_functions_relative_delta_encoding`
- `test_semantic_tokens_full_nonexistent_file_returns_empty_data`
- `test_semantic_tokens_full_overlay_buffer_precedence`
- `test_semantic_tokens_full_overlay_did_change`
- `test_semantic_tokens_full_overlay_did_close`
- `test_semantic_tokens_full_result_id_format`
- `test_semantic_tokens_full_single_function_disk`
- `test_semantic_tokens_range_beyond_eof`
- `test_semantic_tokens_range_bounded_first_function`
- `test_semantic_tokens_range_bounded_second_function`
- `test_semantic_tokens_range_echoes_numeric_id`
- `test_semantic_tokens_range_echoes_string_id`
- `test_semantic_tokens_range_empty_window`

#### `textDocument/foldingRange` (21 e2e tests)
- `test_folding_range_braces_inside_block_comments_ignored`
- `test_folding_range_braces_inside_char_literals_ignored`
- `test_folding_range_braces_inside_line_comments_ignored`
- `test_folding_range_braces_inside_string_literals_ignored`
- `test_folding_range_deeply_nested_five_levels`
- `test_folding_range_dispatcher_echoes_numeric_id`
- `test_folding_range_dispatcher_echoes_string_id`
- `test_folding_range_empty_file_returns_empty_array`
- `test_folding_range_escaped_quotes_inside_strings`
- `test_folding_range_multiple_sequential_functions`
- `test_folding_range_nested_blocks_disk`
- `test_folding_range_nonexistent_file_returns_empty_array`
- `test_folding_range_overlay_dirty_edit_updates_folding`
- `test_folding_range_overlay_scopes_take_precedence`
- `test_folding_range_overlay_single_line_scope_filtered`
- `test_folding_range_single_function_disk_fallback`
- `test_folding_range_single_line_braces_ignored`
- `test_folding_range_struct_impl_and_trait_blocks`
- `test_folding_range_unclosed_braces_handled_safely`
- `test_folding_range_unmatched_closing_braces_ignored`
- `test_l2_folding_range_and_code_lens`

#### `textDocument/selectionRange` (20 e2e tests)
- `test_selection_range_after_did_change_overlay_edit`
- `test_selection_range_cursor_at_eof`
- `test_selection_range_cursor_at_start_of_file`
- `test_selection_range_cursor_between_functions`
- `test_selection_range_cursor_inside_inner_block`
- `test_selection_range_cursor_on_closure_or_loop`
- `test_selection_range_deeply_nested_blocks`
- `test_selection_range_duplicate_positions_in_request`
- `test_selection_range_empty_positions_returns_empty`
- `test_selection_range_missing_positions_key`
- `test_selection_range_multiple_documents_isolation`
- `test_selection_range_multiple_positions_batch`
- `test_selection_range_nested_struct_impl_method`
- `test_selection_range_numeric_request_id_echoed`
- `test_selection_range_parent_nesting_invariants`
- `test_selection_range_position_beyond_file_length`
- `test_selection_range_range_coordinates_validity`
- `test_selection_range_single_position_nested_scope`
- `test_selection_range_string_request_id_echoed`
- `test_selection_range_unopened_document_returns_empty`

#### `textDocument/documentLink / resolve` (20 e2e tests)
- `test_document_link_angle_bracket_delimiter`
- `test_document_link_dispatcher_echoes_numeric_id`
- `test_document_link_dispatcher_echoes_string_id`
- `test_document_link_empty_file_returns_empty`
- `test_document_link_file_without_links_returns_empty`
- `test_document_link_markdown_bracket_and_parenthesis_delimiters`
- `test_document_link_multiple_links_same_line`
- `test_document_link_nonexistent_file_returns_empty`
- `test_document_link_overlay_did_change_updates_links`
- `test_document_link_overlay_did_close_falls_back_to_disk`
- `test_document_link_overlay_precedence_over_disk`
- `test_document_link_quote_delimiter`
- `test_document_link_resolve_echoes_valid_link_params`
- `test_document_link_resolve_missing_params_returns_null`
- `test_document_link_resolve_numeric_request_id_echoed`
- `test_document_link_resolve_string_request_id_echoed`
- `test_document_link_resolve_with_custom_tooltip_and_data`
- `test_document_link_single_https_in_doc_comment`
- `test_document_link_square_bracket_delimiter`
- `test_document_link_utf16_offsets_with_multibyte_prefix`

#### `textDocument/documentColor / colorPresentation (stub)` (20 e2e tests)
- `test_color_presentation_stub_consecutive_queries_isolation`
- `test_color_presentation_stub_dispatcher_echoes_numeric_id`
- `test_color_presentation_stub_dispatcher_echoes_string_id`
- `test_color_presentation_stub_missing_color_object`
- `test_color_presentation_stub_missing_range_object`
- `test_color_presentation_stub_multi_line_range`
- `test_color_presentation_stub_overlay_document_uri`
- `test_color_presentation_stub_rgb_color_params`
- `test_color_presentation_stub_rgba_color_params`
- `test_color_presentation_stub_zero_range`
- `test_document_color_stub_after_did_change_returns_empty_array`
- `test_document_color_stub_after_did_close_returns_empty_array`
- `test_document_color_stub_disk_file_returns_empty_array`
- `test_document_color_stub_dispatcher_echoes_numeric_id`
- `test_document_color_stub_dispatcher_echoes_string_id`
- `test_document_color_stub_empty_file_returns_empty_array`
- `test_document_color_stub_missing_text_document_param`
- `test_document_color_stub_nonexistent_file_returns_empty_array`
- `test_document_color_stub_overlay_buffer_returns_empty_array`
- `test_document_color_stub_sequential_requests_isolation`

#### `textDocument/codeLens / resolve` (21 e2e tests)
- `test_code_lens_disk_workspace_definition`
- `test_code_lens_dispatcher_echoes_numeric_id`
- `test_code_lens_dispatcher_echoes_string_id`
- `test_code_lens_empty_file_returns_empty`
- `test_code_lens_file_without_symbols_returns_empty`
- `test_code_lens_multiple_definitions_in_file`
- `test_code_lens_nonexistent_file_returns_empty`
- `test_code_lens_overlay_did_change_updates_lenses`
- `test_code_lens_overlay_did_close_falls_back_to_index`
- `test_code_lens_overlay_symbols_precedence`
- `test_code_lens_resolve_echoes_numeric_id`
- `test_code_lens_resolve_echoes_string_id`
- `test_code_lens_resolve_missing_data_returns_unchanged`
- `test_code_lens_resolve_overlay_dirty_mapped_coords`
- `test_code_lens_resolve_plural_title_formatting`
- `test_code_lens_resolve_roundtrip_from_code_lens_request`
- `test_code_lens_resolve_singular_title_formatting`
- `test_code_lens_resolve_with_multiple_references`
- `test_code_lens_resolve_with_single_reference`
- `test_code_lens_resolve_with_zero_references`
- `test_l2_code_lens_command_declared_in_execute_command_provider`

### Domain 8: Editing & Workspace Edits

#### `textDocument/prepareRename` (0 e2e tests)
_No classified e2e tests yet._

#### `textDocument/rename` (5 e2e tests)
- `test_case_10_stale_rename_overlay_resolution`
- `test_case_8_external_dependency_rename_safety`
- `test_ntm_overlay_mutations_and_editing_pipeline`
- `test_refactor_cross_file_workspace_rename_execution`
- `test_refactor_rename_local_variable_execution`

#### `textDocument/linkedEditingRange` (1 e2e tests)
- `test_l2_linked_editing_range_local_bindings`

#### `textDocument/formatting / rangeFormatting` (3 e2e tests)
- `test_l2_formatting_full_and_idempotence`
- `test_l2_range_formatting_scoped`
- `test_refactor_formatting_and_range_formatting_execution`

#### `textDocument/onTypeFormatting` (1 e2e tests)
- `test_l2_on_type_formatting_delegation`

#### `workspace/executeCommand` (14 e2e tests)
- `test_apply_edit_two_way_handshake_reject_and_accept`
- `test_apply_workspace_edit_with_newtext`
- `test_client_ack_does_not_rewrite_closed_file_on_disk`
- `test_client_advancing_to_version_2_rejects_version_1_apply_edit`
- `test_combined_versions_retain_initial_client_snapshot`
- `test_domain4_stream_execute_command`
- `test_l2_workspace_execute_commands`
- `test_multiple_arguments_preserve_all_edits`
- `test_open_document_command_edits_sync_disk_and_return_payload`
- `test_outbound_apply_edit_preserves_document_changes_version`
- `test_partial_workspace_edit_rejected_atomically`
- `test_same_position_inserts_preserve_declared_order`
- `test_sequential_workspace_edit_client_snapshot_consistency`
- `test_two_concurrent_pending_edits_distinct_ids`

### Domain 9: Navigation & Symbols

#### `textDocument/definition / declaration (delegating)` (44 e2e tests)
- `test_case_1_associated_type_navigation`
- `test_case_2_trait_method_concrete_vs_generic`
- `test_case_3_two_versions_of_same_crate`
- `test_case_4_exact_ident_span_and_raw_idents`
- `test_case_5_macros_and_derives_navigation`
- `test_case_6_field_shorthand_and_destructuring`
- `test_case_7_aliased_use_and_reexports`
- `test_case_9_line_shift_definition_and_references`
- `test_case_cross_crate_ident_name_isolation`
- `test_case_sysroot_atomic_tyalias_navigation_and_refs`
- `test_goto_def_and_rename_generic_type_param_t`
- `test_l1_declaration_cross_file_same_crate`
- `test_l1_declaration_definition_parity_contract`
- `test_l1_declaration_matches_definition_same_file`
- `test_l1_definition_cross_crate_two_shards`
- `test_l1_definition_cross_file_same_crate`
- `test_l1_definition_cursor_on_whitespace_returns_null`
- `test_l1_definition_empty_params_returns_null_safe`
- `test_l1_definition_in_same_file`
- `test_l1_definition_modified_imports_skips_workspace_index`
- `test_l1_definition_out_of_bounds_returns_null`
- `test_l1_definition_same_named_symbol_in_two_crates_resolves_referenced_target`
- `test_l1_definition_target_file_dirty_overlay_definition_deleted_returns_null`
- `test_l1_definition_target_file_in_dirty_overlay_mapped_live_range`
- `test_l1_definition_type_alias_synthetic`
- `test_l1_definition_unregistered_file_uri_returns_null`
- `test_l1_definition_utf16_multibyte_prefix_with_emoji`
- `test_l1_overlay_preserves_navigation`
- `test_l2_goto_definition_inside_cfg_test_method`
- `test_l2_goto_definition_inside_cfg_test_struct`
- `test_l2_macro_included_file_navigation`
- `test_l2_macro_multiplier_navigation_and_refs`
- `test_l2_multiroot_cross_crate_navigation`
- `test_l2_reexport_aliased_type_navigation`
- `test_l2_reexport_definition_resolution`
- `test_l2_reexport_item_definition_and_declaration`
- `test_l2_reexport_statement_navigation`
- `test_l2_struct_field_shorthand_navigation_and_refs`
- `test_l2_sysroot_atomic_load_definition`
- `test_l2_sysroot_ops_range_navigation`
- `test_l2_traits_call_generic_method_definition`
- `test_ntm_end_to_end_lsp_pipeline`
- `test_ntm_hierarchy_and_protocol_invariants`
- `test_staleness_definition_in_modified_hunk_abstains`

#### `textDocument/typeDefinition` (20 e2e tests)
- `test_l1_type_definition_cross_crate_two_shards`
- `test_l1_type_definition_cross_file`
- `test_l1_type_definition_dirty_overlay_returns_null`
- `test_l1_type_definition_enum_binding`
- `test_l1_type_definition_field_access_expression`
- `test_l1_type_definition_fn_parameter_cross_file`
- `test_l1_type_definition_generic_wrapper_type_string_returns_null`
- `test_l1_type_definition_local_variable_with_overlay`
- `test_l1_type_definition_missing_from_type_table_returns_null`
- `test_l1_type_definition_no_exact_match_returns_null`
- `test_l1_type_definition_prefers_exact_name_over_substring_match`
- `test_l1_type_definition_qualified_path_exact_match`
- `test_l1_type_definition_qualified_path_suffix_fallback`
- `test_l1_type_definition_same_file_wins_over_other_file`
- `test_l1_type_definition_same_named_type_prefers_local_crate`
- `test_l1_type_definition_single_file`
- `test_l1_type_definition_struct_field_target`
- `test_l1_type_definition_utf16_query_side`
- `test_l1_type_definition_utf16_with_emoji_prefix`
- `test_l1_type_definition_whitespace_and_oob_bounds`

#### `textDocument/implementation` (16 e2e tests)
- `test_l1_implementation_blanket_impl_indexed_ref`
- `test_l1_implementation_cross_crate_two_shards`
- `test_l1_implementation_cursor_on_impl_header_returns_self_impl`
- `test_l1_implementation_cursor_on_trait_method_returns_empty`
- `test_l1_implementation_dirty_overlay_and_boundaries`
- `test_l1_implementation_dirty_overlay_returns_empty`
- `test_l1_implementation_generic_impl_block`
- `test_l1_implementation_generic_impl_indexed_ref`
- `test_l1_implementation_multiple_implementations`
- `test_l1_implementation_struct_inherent_and_two_traits`
- `test_l1_implementation_trait_cross_file`
- `test_l1_implementation_trait_single_file`
- `test_l1_implementation_trait_without_implementations`
- `test_l1_implementation_two_separate_files_same_crate`
- `test_l1_implementation_utf16_multibyte_prefix_columns`
- `test_l1_implementation_utf16_with_emoji_prefix`

#### `textDocument/references` (20 e2e tests)
- `test_l1_references_cross_crate_two_shards`
- `test_l1_references_cross_file`
- `test_l1_references_empty_params_safe`
- `test_l1_references_local_parameter_without_overlay_returns_empty`
- `test_l1_references_missing_context_or_key_defaults_to_include_decl`
- `test_l1_references_multiple_references_same_line`
- `test_l1_references_on_callsite_finds_all_references`
- `test_l1_references_out_of_bounds_coords_returns_empty`
- `test_l1_references_overlay_dirty_mapped_references`
- `test_l1_references_overlay_local_bindings`
- `test_l1_references_reexport_via_use_same_crate`
- `test_l1_references_same_file_multiple_usages`
- `test_l1_references_trait_method_indexed_refs`
- `test_l1_references_unregistered_uri_returns_empty`
- `test_l1_references_unused_symbol_returns_empty_without_decl_and_one_with_decl`
- `test_l1_references_utf16_multibyte_prefix_columns`
- `test_l2_cross_crate_dispatch_and_references`
- `test_l2_cross_crate_reexport_references_and_rename`
- `test_l2_raw_ident_navigation_and_references`
- `test_l2_traits_default_method_references`

#### `textDocument/documentHighlight` (19 e2e tests)
- `test_l1_document_highlight_boundaries_and_empty`
- `test_l1_document_highlight_clean_overlay_input_column_via_overlay_conversion`
- `test_l1_document_highlight_compiled_struct_field_self_access`
- `test_l1_document_highlight_cross_file_excluded`
- `test_l1_document_highlight_cursor_at_end_col`
- `test_l1_document_highlight_cursor_on_compiled_reference_includes_decl`
- `test_l1_document_highlight_dirty_overlay_skips_compiled_index_returns_empty`
- `test_l1_document_highlight_function_parameter`
- `test_l1_document_highlight_kinds_contract_decl_and_usage`
- `test_l1_document_highlight_let_mut_reassignment`
- `test_l1_document_highlight_local_binding`
- `test_l1_document_highlight_multi_line_references`
- `test_l1_document_highlight_overlay_local_binding_takes_precedence_over_compiled`
- `test_l1_document_highlight_same_line_multiple_occurrences`
- `test_l1_document_highlight_shadowed_local_binding`
- `test_l1_document_highlight_symbol_in_macro_invocation`
- `test_l1_document_highlight_target_zero_end_col_fallback_symbol_len`
- `test_l1_document_highlight_utf16_multibyte_declaration_and_references`
- `test_l1_document_highlight_workspace_references`

#### `textDocument/documentSymbol` (16 e2e tests)
- `test_case_11_new_symbol_in_live_buffer`
- `test_l1_document_symbol_all_kinds`
- `test_l1_document_symbol_const_static_type_alias`
- `test_l1_document_symbol_crlf_preservation`
- `test_l1_document_symbol_did_change_dynamic_refresh`
- `test_l1_document_symbol_enum_structure`
- `test_l1_document_symbol_flat_list_no_children`
- `test_l1_document_symbol_function_signature_detail_exact`
- `test_l1_document_symbol_mod_blocks`
- `test_l1_document_symbol_multi_document_isolation`
- `test_l1_document_symbol_nested_methods`
- `test_l1_document_symbol_selection_range_exact`
- `test_l1_document_symbol_trait_with_methods`
- `test_l1_document_symbol_typaram_filtered`
- `test_l1_document_symbol_unicode_ranges`
- `test_l1_document_symbol_unopened_and_empty`

#### `workspace/symbol` (20 e2e tests)
- `test_l1_workspace_symbol_cap_100_across_shards`
- `test_l1_workspace_symbol_cap_100_single_shard_boundary`
- `test_l1_workspace_symbol_case_insensitive`
- `test_l1_workspace_symbol_cross_crate_same_name`
- `test_l1_workspace_symbol_deep_module_hierarchy_container_name`
- `test_l1_workspace_symbol_empty_query_lists_all`
- `test_l1_workspace_symbol_exact_match`
- `test_l1_workspace_symbol_kind_mapping`
- `test_l1_workspace_symbol_missing_or_null_query_param_defaults_to_all`
- `test_l1_workspace_symbol_multiple_indices_merged_in_order_and_deduplicated`
- `test_l1_workspace_symbol_no_matches_returns_empty`
- `test_l1_workspace_symbol_non_ascii_german_query`
- `test_l1_workspace_symbol_path_matching`
- `test_l1_workspace_symbol_same_index_twice_deduplicated`
- `test_l1_workspace_symbol_submodule_path_query_matching`
- `test_l1_workspace_symbol_typaram_excluded`
- `test_l1_workspace_symbol_utf16_location_with_emoji_prefix`
- `test_l1_workspace_symbol_whitespace_padded_query_returns_empty`
- `test_l2_multiroot_symbols_search_across_crates`
- `test_l2_workspace_symbol_search_and_deduplication`

#### `workspaceSymbol/resolve` (15 e2e tests)
- `test_l1_workspace_symbol_resolve_already_populated`
- `test_l1_workspace_symbol_resolve_bare_path_empty_container_name`
- `test_l1_workspace_symbol_resolve_crate_root_container_name`
- `test_l1_workspace_symbol_resolve_cross_shard_multi_crate`
- `test_l1_workspace_symbol_resolve_exact_match_priority`
- `test_l1_workspace_symbol_resolve_explicit_null_location_substitutes`
- `test_l1_workspace_symbol_resolve_missing_location`
- `test_l1_workspace_symbol_resolve_nested_module_container_name`
- `test_l1_workspace_symbol_resolve_non_ascii_symbol_name`
- `test_l1_workspace_symbol_resolve_nonexistent_returns_as_is`
- `test_l1_workspace_symbol_resolve_null_params_safe`
- `test_l1_workspace_symbol_resolve_preserves_custom_client_payload`
- `test_l1_workspace_symbol_resolve_preserves_kind_with_location_sub`
- `test_l1_workspace_symbol_resolve_same_name_multiple_crates_first_shard`
- `test_l2_workspace_symbol_resolve`

### Domain 10: Multi-root & File Events

#### `workspace/didChangeWorkspaceFolders` (21 e2e tests)
- `test_did_change_workspace_folders_add_multiple_roots_batch`
- `test_did_change_workspace_folders_add_single_root`
- `test_did_change_workspace_folders_all_workspaces_order_preserved`
- `test_did_change_workspace_folders_dispatched_via_lsp_message`
- `test_did_change_workspace_folders_duplicate_addition_ignored`
- `test_did_change_workspace_folders_empty_added_and_removed_is_noop`
- `test_did_change_workspace_folders_empty_params_safe`
- `test_did_change_workspace_folders_empty_uri_entry_skipped`
- `test_did_change_workspace_folders_invalid_file_uri_skipped`
- `test_did_change_workspace_folders_malformed_added_entry_not_object`
- `test_did_change_workspace_folders_malformed_removed_entry_not_object`
- `test_did_change_workspace_folders_missing_event_object_is_noop`
- `test_did_change_workspace_folders_non_file_scheme_skipped`
- `test_did_change_workspace_folders_nonexistent_path_skipped`
- `test_did_change_workspace_folders_remove_all_roots_via_event`
- `test_did_change_workspace_folders_remove_multiple_roots_batch`
- `test_did_change_workspace_folders_remove_nonexistent_root_is_noop`
- `test_did_change_workspace_folders_remove_single_root`
- `test_did_change_workspace_folders_routing_switches_after_removal`
- `test_did_change_workspace_folders_simultaneous_add_and_remove`
- `test_l2_multiroot_dynamic_workspace_folders`

#### `workspace/didChangeWatchedFiles` (20 e2e tests)
- `test_did_change_watched_files_active_overlay_preserved_across_reload`
- `test_did_change_watched_files_clears_position_converter_cache`
- `test_did_change_watched_files_corrupted_shard_preserves_old_index`
- `test_did_change_watched_files_deleted_definition_removed_after_reload`
- `test_did_change_watched_files_deleted_shard_removed_from_workspace`
- `test_did_change_watched_files_dispatch_returns_true`
- `test_did_change_watched_files_empty_changes_array_still_reloads_disk`
- `test_did_change_watched_files_event_type_1_created`
- `test_did_change_watched_files_event_type_2_changed`
- `test_did_change_watched_files_event_type_3_deleted`
- `test_did_change_watched_files_manifest_variant_switch_reloads_shards`
- `test_did_change_watched_files_missing_params_handled_safely`
- `test_did_change_watched_files_multiple_shards_only_one_modified`
- `test_did_change_watched_files_multiple_workspaces_all_reloaded`
- `test_did_change_watched_files_new_definition_discoverable_after_reload`
- `test_did_change_watched_files_new_shard_discovered_in_workspace_dir`
- `test_did_change_watched_files_rapid_successive_notifications`
- `test_did_change_watched_files_reloads_symbols_exact_count`
- `test_did_change_watched_files_single_shard_crate_version_reloaded`
- `test_did_change_watched_files_unmodified_shard_preserves_cached_state`

#### `workspace/didCreateFiles / willCreateFiles` (20 e2e tests)
- `test_did_create_files_batch_twenty_files`
- `test_did_create_files_empty_files_noop`
- `test_did_create_files_empty_uri_skipped`
- `test_did_create_files_file_outside_roots_ignored`
- `test_did_create_files_followed_by_did_open`
- `test_did_create_files_missing_files_key_noop`
- `test_did_create_files_multi_root_sync`
- `test_did_create_files_open_dirty_buffer_not_overwritten`
- `test_did_create_files_percent_encoded_uri`
- `test_did_create_files_relative_path_resolution`
- `test_did_create_files_retains_existing_roots`
- `test_did_create_files_single_file_clears_conv`
- `test_did_create_files_unmodified_index_preserves_baseline`
- `test_did_create_files_with_updated_disk_index_syncs`
- `test_will_create_files_empty_files_array`
- `test_will_create_files_multi_file_returns_empty_edit`
- `test_will_create_files_numeric_id_preserved`
- `test_will_create_files_returns_matching_schema`
- `test_will_create_files_string_id_preserved`
- `test_will_create_files_valid_file_returns_empty_edit`

#### `workspace/didRenameFiles / willRenameFiles` (20 e2e tests)
- `test_did_rename_files_active_overlay_rebound`
- `test_did_rename_files_chain_rename`
- `test_did_rename_files_clears_position_converter`
- `test_did_rename_files_cyclic_rename_roundtrip`
- `test_did_rename_files_dirty_overlay_preserves_text_and_version`
- `test_did_rename_files_empty_files_noop`
- `test_did_rename_files_followed_by_did_change`
- `test_did_rename_files_followed_by_did_close`
- `test_did_rename_files_missing_fields_skipped`
- `test_did_rename_files_missing_files_key_noop`
- `test_did_rename_files_multi_rename_simultaneous`
- `test_did_rename_files_multi_root_secondary_workspace`
- `test_did_rename_files_percent_encoded_uri`
- `test_did_rename_files_unopened_file_noop`
- `test_will_rename_files_empty_files_array`
- `test_will_rename_files_lifecycle_sequence_with_did_rename`
- `test_will_rename_files_multi_file_returns_empty_edit`
- `test_will_rename_files_numeric_id_preserved`
- `test_will_rename_files_string_id_preserved`
- `test_will_rename_files_valid_returns_empty_edit`

#### `workspace/didDeleteFiles / willDeleteFiles` (20 e2e tests)
- `test_did_delete_files_active_overlay_evicted`
- `test_did_delete_files_batch_ten_files_partial_open`
- `test_did_delete_files_clears_position_converter`
- `test_did_delete_files_dirty_overlay_discarded`
- `test_did_delete_files_empty_files_noop`
- `test_did_delete_files_idempotent_multiple_deletions`
- `test_did_delete_files_missing_files_key_noop`
- `test_did_delete_files_missing_uri_field_skipped`
- `test_did_delete_files_multi_file_simultaneous`
- `test_did_delete_files_multi_root_secondary_workspace`
- `test_did_delete_files_nonexistent_path_noop`
- `test_did_delete_files_percent_encoded_uri`
- `test_did_delete_files_reopen_after_deletion`
- `test_did_delete_files_unopened_file_noop`
- `test_will_delete_files_empty_files_array`
- `test_will_delete_files_lifecycle_sequence_with_did_delete`
- `test_will_delete_files_multi_file_returns_empty_edit`
- `test_will_delete_files_numeric_id_preserved`
- `test_will_delete_files_string_id_preserved`
- `test_will_delete_files_valid_file_returns_empty_edit`

### Unclassified E2E Tests

Total unclassified tests: 82 (multi-purpose, property, or helper harness tests)

- `test_atomic_closed_write_preserves_large_file_on_error`
- `test_atomic_write_preserves_file_permissions`
- `test_atomic_write_preserves_symlink_referent`
- `test_auto_import_appends_after_existing_use_statements`
- `test_auto_import_detects_already_imported_symbols`
- `test_auto_import_detects_braced_spaces_and_glob_imports`
- `test_auto_import_inserts_after_multiline_use_not_inside`
- `test_auto_import_skips_inner_attributes_and_comments`
- `test_auto_import_skips_prelude_and_bare_names`
- `test_auto_import_standard_collection`
- `test_byte_offset_to_position_roundtrip`
- `test_char_col_to_utf16_unit_boundaries`
- `test_cli_fixture_generation_and_query`
- `test_comparison_preserves_full_path_and_rejects_errors`
- `test_container_matching_exact_components`
- `test_disk_failure_rollback_and_reported_failure`
- `test_domain4_pre_initialize_requests_rejected_with_32002`
- `test_domain5_concurrent_intelligence_requests_stress`
- `test_domain5_e2e_full_intelligence_stream`
- `test_domain5_pre_initialize_requests_rejected_with_32002`
- `test_find_ridx_driver_custom_env_var`
- `test_find_ridx_driver_fallback_candidate_search`
- `test_find_ridx_driver_non_existent_returns_none`
- `test_has_modified_imports_add_remove_change`
- `test_hermetic_scoping_module_visibility_and_use_std_fmt`
- `test_hermetic_scoping_shadowing_and_reverse_sysroot`
- `test_l2_cas_portability_and_closure_sanitization`
- `test_l2_edition_resolution`
- `test_l2_hierarchy_latency_benchmark`
- `test_l2_macro_expansion_no_synthetic_bindings`
- `test_l2_multiroot_path_routing_longest_prefix`
- `test_l2_multiroot_provenance_and_source_hash`
- `test_l2_presentation_colors_and_links`
- `test_l2_reexport_unmerged_safety_refusal`
- `test_l2_std_collections_hashmap_lookup`
- `test_l2_workspace_file_operations_lifecycle`
- `test_map_compiled_line_to_live_after_delete`
- `test_map_compiled_line_to_live_after_insert`
- `test_map_compiled_to_live_position_tuple`
- `test_map_line_identical_buffer`
- `test_map_line_modified_middle_hunk_returns_none`
- `test_map_line_no_compiled_text_returns_none`
- `test_map_line_overlapping_prefix_suffix_boundary`
- `test_map_line_prefix_common_unaffected`
- `test_map_line_single_line_file`
- `test_map_line_suffix_common_shifted_after_delete`
- `test_map_line_suffix_common_shifted_after_insert`
- `test_map_live_to_compiled_position_tuple`
- `test_multiroot_api_clear_removes_all_roots`
- `test_multiroot_api_doc_ws_empty_uri_falls_back`
- `test_multiroot_api_doc_ws_matches_root_file`
- `test_multiroot_api_find_workspace_on_empty_is_none`
- `test_multiroot_api_reopen_from_slice`
- `test_overlay_manager_open_count_and_file_paths`
- `test_overlay_rename_document_path`
- `test_position_to_byte_offset_ascii_lines`
- `test_position_to_byte_offset_multibyte_greek`
- `test_position_to_byte_offset_surrogate_pair_emoji`
- `test_prelude_allows_fmt_result_and_requires_hash`
- `test_proptest_mapping_monotonicity_invariant`
- `test_proptest_mapping_never_panics_on_arbitrary_inputs`
- `test_proptest_mapping_roundtrip_bijectivity`
- `test_proptest_multibyte_random_edits_utf8_validity`
- `test_proptest_random_incremental_edits_vs_oracle`
- `test_review_v4651_closed_edit_remains_dirty`
- `test_review_v4651_dual_edit_representation_applied_once`
- `test_review_v4651_edits_use_original_coordinates`
- `test_review_v4651_harness_routes_response_ids_and_preserves_errors`
- `test_review_v4651_stale_version_rejected`
- `test_run_server_session_handles_invalid_binary_gracefully`
- `test_same_crate_auto_import_produces_crate_prefix`
- `test_scip_loader_and_queries`
- `test_server_request_with_same_id_is_not_treated_as_response`
- `test_set_metrics_precision_recall_f1`
- `test_staleness_index_reload_resyncs_overlay`
- `test_staleness_query_on_deleted_file`
- `test_symbol_resolution_chooses_exact_candidate`
- `test_symmetric_runner_records_rss_and_graceful_shutdown`
- `test_symmetric_scenario_initialization`
- `test_symmetric_summary_serialization_roundtrip`
- `test_utf16_col_to_char_col_and_char_to_utf16`
- `test_workspace_index_source_root_at`

---
coverage: e2e 706 total, 20 rows >= 20, 1 rows >= 50 of 53
