/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Aggregated end-to-end and integration test suite for Rust Intelligence (ridx).

mod cli;
mod domain2_compiler_message;
mod domain2_flycheck_lifecycle;
mod domain2_flycheck_state;
mod domain2_pull_diagnostics;
mod domain2_real_compiler_fixtures;
mod domain2_shifted_diagnostics;
mod domain2_syntax_diagnostics;
mod domain3_coordinates;
mod domain3_proptest;
mod domain3_staleness;
mod domain3_sync_edits;
mod domain3_sync_lifecycle;
mod domain3_sync_open;
mod l2_cases;
mod l2_harness;
mod ntm_compare_harness;
mod ntm_domain1_cancel_request;
mod ntm_domain1_cancel_request_part2;
mod ntm_domain1_exit;
mod ntm_domain1_exit_part2;
mod ntm_domain1_initialize;
mod ntm_domain1_initialize_part2;
mod ntm_domain1_set_trace;
mod ntm_domain1_set_trace_part2;
mod ntm_domain1_shutdown;
mod ntm_domain1_shutdown_part2;
mod ntm_domain2_document_diagnostic;
mod ntm_domain2_workspace_diagnostic;
mod ntm_domain10_file_create;
mod ntm_domain10_file_delete;
mod ntm_domain10_file_rename;
mod ntm_domain10_l2_multiroot;
mod ntm_domain10_watched_files;
mod ntm_domain10_workspace_folders;
mod ntm_domain4_actions_stress;
mod ntm_domain5_intelligence_stress;
mod ntm_domain5_l2_cas_and_macros;
mod ntm_domain5_l2_cfg_test;
mod ntm_domain5_l2_intelligence;
mod ntm_domain5_scoping;
mod ntm_domain5_shadowing;
mod ntm_domain6_incoming_calls;
mod ntm_domain6_incoming_calls_part2;
mod ntm_domain6_incoming_calls_part3;
mod ntm_domain6_l2_hierarchies;
mod ntm_domain6_outgoing_calls;
mod ntm_domain6_outgoing_calls_part2;
mod ntm_domain6_outgoing_calls_part3;
mod ntm_domain6_prepare_call;
mod ntm_domain6_prepare_call_part2;
mod ntm_domain6_prepare_call_part3;
mod ntm_domain6_prepare_type;
mod ntm_domain6_prepare_type_part2;
mod ntm_domain6_prepare_type_part3;
mod ntm_domain6_subtypes;
mod ntm_domain6_subtypes_part2;
mod ntm_domain6_subtypes_part3;
mod ntm_domain6_subtypes_part4;
mod ntm_domain6_supertypes;
mod ntm_domain6_supertypes_part2;
mod ntm_domain6_supertypes_part3;
mod ntm_domain6_supertypes_part4;
mod ntm_domain7_code_lens;
mod ntm_domain7_code_lens_part2;
mod ntm_domain7_color;
mod ntm_domain7_color_part2;
mod ntm_domain7_document_link;
mod ntm_domain7_document_link_part2;
mod ntm_domain7_folding;
mod ntm_domain7_folding_part2;
mod ntm_domain7_l2_presentation;
mod ntm_domain7_selection_range;
mod ntm_domain7_selection_range_part2;
mod ntm_domain7_semantic_tokens;
mod ntm_domain7_semantic_tokens_part2;
mod ntm_domain8_l2_editing;
mod ntm_domain9_l1_definition;
mod ntm_domain9_l1_definition_part2;
mod ntm_domain9_l1_highlight_and_resolve;
mod ntm_domain9_l1_highlight_part2;
mod ntm_domain9_l1_implementation_part2;
mod ntm_domain9_l1_implementation_part3;
mod ntm_domain9_l1_references;
mod ntm_domain9_l1_references_part2;
mod ntm_domain9_l1_references_part3;
mod ntm_domain9_l1_symbols_highlight;
mod ntm_domain9_l1_symbols_part2;
mod ntm_domain9_l1_symbols_part3;
mod ntm_domain9_l1_type_and_impl;
mod ntm_domain9_l1_type_definition_part2;
mod ntm_domain9_l1_type_definition_part3;
mod ntm_domain9_l1_workspace_symbols;
mod ntm_domain9_l1_workspace_symbols_part2;
mod ntm_domain9_l1_workspace_symbols_part3;
mod ntm_domain9_l1_workspace_symbols_part4;
mod ntm_domain9_l2_compiler_nav;
mod ntm_domain9_l2_workspace;
mod ntm_e2e;
mod ntm_flycheck_and_auto_import;
mod ntm_formatting_execution;
mod ntm_hierarchy_stress;
mod ntm_overlay_mutation;
mod ntm_refactoring_execution;
mod ntm_rename_execution;
mod ntm_review_repro;
mod ntm_symmetric_runner;
mod ntm_version_and_atomic_publish;
mod ntm_version_precondition;
mod ntm_workspace_edit;
mod scip_loader;
