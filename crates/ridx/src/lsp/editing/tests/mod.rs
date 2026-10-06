/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

pub mod format_tests;
pub mod rename_tests;

use crate::lsp::protocol::PositionConverter;
use crate::overlay::OverlayManager;
use ridx_format::{IndexWriter, RawDefinition, WorkspaceIndex};
use std::fs;
use std::path::PathBuf;

static EDIT_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn setup_test_editing() -> (PathBuf, WorkspaceIndex, OverlayManager, PositionConverter) {
    let id = EDIT_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp_dir =
        std::env::temp_dir().join(format!("ridx_test_edit_{}_{}", std::process::id(), id));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let code = "pub fn add_values(x: i32, y: i32) -> i32 {\n    let sum = x + y;\n    sum\n}\n";
    let _ = fs::write(&src_file, code);

    let src_file_str = src_file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("test_edit", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: ridx_format::DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: src_file_str.clone(),
        name: "add_values".to_string(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 18,
        path: "test_edit::add_values".to_string(),
        signature: Some("fn add_values(x: i32, y: i32) -> i32".to_string()),
    });

    writer.add_reference(ridx_format::RawReference {
        file: src_file_str.clone(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 18,
        target_def_id: Some(1),
        target_path: "test_edit::add_values".to_string(),
        target_unit: None,
        target_file: Some(src_file_str.clone()),
        target_line: 1,
        target_col: 8,
        kind: ridx_format::DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });

    let caller_file = temp_dir.join("src/caller.rs");
    let caller_code = "pub fn call() { test_edit::add_values(1, 2); }\n";
    let _ = fs::write(&caller_file, caller_code);
    let caller_str = caller_file.to_str().unwrap().to_string();

    writer.add_reference(ridx_format::RawReference {
        file: caller_str,
        start_line: 1,
        start_col: 17,
        end_line: 1,
        end_col: 27,
        target_def_id: Some(1),
        target_path: "test_edit::add_values".to_string(),
        target_unit: None,
        target_file: Some(src_file_str),
        target_line: 1,
        target_col: 8,
        kind: ridx_format::DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });

    let ridx_file = temp_dir.join("index.ridx");
    let mut f = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();

    let ws = WorkspaceIndex::open(&ridx_file).unwrap();
    let overlay_mgr = OverlayManager::new();
    let conv = PositionConverter::new();

    (temp_dir, ws, overlay_mgr, conv)
}
