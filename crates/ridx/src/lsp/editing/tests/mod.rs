/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

pub mod code_action_assists_tests;
pub mod code_action_expanded_tests;
pub mod code_action_inline_callers_args_tests;
pub mod code_action_inline_callers_generic_tests;
pub mod code_action_inline_callers_self_tests;
pub mod code_action_inline_callers_tests;
pub mod code_action_inline_callers_try_tests;
pub mod code_action_resolve_tests;
pub mod code_action_struct_where_tests;
pub mod code_action_tests;
pub mod format_tests;
pub mod formatting_unit_tests;
pub mod ident_validation_tests;
pub mod linked_editing_tests;
pub mod on_type_formatting_tests;
pub mod prepare_rename_tests;
pub mod rename_edge_tests;
pub mod rename_general_tests;
pub mod rename_macro_tests;
pub mod rename_matrix_tests;
pub mod rename_prefix_owner_tests;
pub mod rename_sibling_tests;
pub mod rename_sysroot_tests;
pub mod rename_tests;
pub mod rename_type_span_tests;
pub mod rename_unit_tests;

use crate::lsp::protocol::PositionConverter;
use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use std::fs;
use std::path::PathBuf;

static EDIT_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn setup_test_editing() -> (PathBuf, WorkspaceIndex, OverlayManager, PositionConverter) {
    let id = EDIT_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_edit_{}_{}", std::process::id(), id));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let caller_file = temp_dir.join("src/caller.rs");

    let src_file_str = src_file.to_str().unwrap().to_string();
    let caller_str = caller_file.to_str().unwrap().to_string();

    let mut builder = ridx_test_support::FixtureIndexBuilder::new("test_edit", "0.1.0");
    let (code, def_ids) =
        builder.add_defs_with_ids("pub fn «add_values»(x: i32, y: i32) -> i32 {\n    let sum = x + y;\n    sum\n}\n", &src_file_str, "test_edit", ridx_format::DefKind::Fn);
    let _ = fs::write(&src_file, code);

    // Self-reference at declaration site
    builder.add_ref("test_edit::add_values", &src_file_str, 1, 8, 1, 18, Some(def_ids[0]), Some(&src_file_str), 1, 8, ridx_format::DefKind::Fn);

    let caller_code = "pub fn call() { test_edit::add_values(1, 2); }\n";
    let _ = fs::write(&caller_file, caller_code);
    builder.add_ref("test_edit::add_values", &caller_str, 1, 17, 1, 27, Some(def_ids[0]), Some(&src_file_str), 1, 8, ridx_format::DefKind::Fn);

    let ridx_file = temp_dir.join("index.ridx");
    builder.write_to_file(&ridx_file);

    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let overlay_mgr = OverlayManager::new();
    let conv = PositionConverter::new();

    (temp_dir, ws, overlay_mgr, conv)
}
