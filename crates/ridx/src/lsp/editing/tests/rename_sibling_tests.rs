/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Tests for symbol renaming across sibling workspace units (lib and lib test).
//!
//! Verifies that when a symbol is renamed across both a library shard and its
//! test shard, TextEdits are strictly deduplicated, no overlapping edits occur,
//! and references in sibling test shards are accurately renamed.

use crate::lsp::editing::rename::handle_rename;
use crate::lsp::protocol::{path_to_uri, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::writer::{IndexWriter, RawDefinition, RawReference};
use ridx_format::{DefKind, WorkspaceIndex};
use ridx_test_support::apply_workspace_edit;
use serde_json::json;
use std::collections::HashMap;
use std::fs;

#[test]
fn test_rename_sibling_units_deduplicated_workspace_edit() {
    let temp_dir = tempfile::tempdir().unwrap();
    let ws_root = temp_dir.path().to_path_buf();
    let pkg_root = ws_root.join("my_pkg");
    fs::create_dir_all(pkg_root.join("src")).unwrap();
    fs::create_dir_all(pkg_root.join("tests")).unwrap();

    let lib_rs = pkg_root.join("src/lib.rs");
    let test_rs = pkg_root.join("tests/test_run.rs");

    let lib_str = lib_rs.to_str().unwrap().to_string();
    let test_str = test_rs.to_str().unwrap().to_string();

    let lib_content = "pub fn compute(x: i32) -> i32 {\n    x * 2\n}\n\npub fn run() -> i32 {\n    compute(10)\n}\n";
    let test_content = "pub fn verify() -> i32 {\n    compute(20)\n}\n";

    fs::write(&lib_rs, lib_content).unwrap();
    fs::write(&test_rs, test_content).unwrap();

    let variants_dir = ws_root.join("target/ridx/variants/dev");
    let deps_dir = variants_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();

    // 1. Lib shard writer
    let mut lib_writer = IndexWriter::new("my_pkg", "0.1.0");
    lib_writer.set_unit_key("unit_lib");
    lib_writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "compute".to_string(),
        path: "my_pkg::compute".to_string(),
        signature: Some("fn(i32) -> i32".to_string()),
        file: lib_str.clone(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 15,
    });
    lib_writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 6,
        start_col: 5,
        end_line: 6,
        end_col: 12,
        target_def_id: Some(1),
        target_path: "my_pkg::compute".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 8,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    let lib_idx_path = variants_dir.join("my_pkg_lib.ridx");
    let mut f_lib = fs::File::create(&lib_idx_path).unwrap();
    lib_writer.write_to(&mut f_lib).unwrap();
    drop(f_lib);

    let lib_meta = json!({
        "name": "my_pkg",
        "version": "0.1.0",
        "unit_key": "unit_lib",
        "source_root": pkg_root.to_str().unwrap(),
        "target_kind": "lib",
        "is_dependency": false
    });
    fs::write(deps_dir.join("my_pkg_lib.json"), serde_json::to_string(&lib_meta).unwrap()).unwrap();

    // 2. Lib test shard writer (same package, duplicates lib definitions/references + adds test reference)
    let mut test_writer = IndexWriter::new("my_pkg", "0.1.0");
    test_writer.set_unit_key("unit_test");
    test_writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "compute".to_string(),
        path: "my_pkg::compute".to_string(),
        signature: Some("fn(i32) -> i32".to_string()),
        file: lib_str.clone(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 15,
    });
    // Duplicate reference in common src/lib.rs
    test_writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 6,
        start_col: 5,
        end_line: 6,
        end_col: 12,
        target_def_id: Some(1),
        target_path: "my_pkg::compute".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 8,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    // Additional reference only in tests/test_run.rs
    test_writer.add_reference(RawReference {
        file: test_str.clone(),
        start_line: 2,
        start_col: 5,
        end_line: 2,
        end_col: 12,
        target_def_id: Some(1),
        target_path: "my_pkg::compute".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 8,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    let test_idx_path = variants_dir.join("my_pkg_test.ridx");
    let mut f_test = fs::File::create(&test_idx_path).unwrap();
    test_writer.write_to(&mut f_test).unwrap();
    drop(f_test);

    let test_meta = json!({
        "name": "my_pkg",
        "version": "0.1.0",
        "unit_key": "unit_test",
        "source_root": pkg_root.to_str().unwrap(),
        "target_kind": "test",
        "is_dependency": false
    });
    fs::write(deps_dir.join("my_pkg_test.json"), serde_json::to_string(&test_meta).unwrap()).unwrap();

    // 3. Variant manifest
    let manifest = json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "dev",
        "variant_dir": "variants/dev",
        "workspace_root": ws_root.to_str().unwrap()
    });
    fs::write(ws_root.join("target/ridx/ridx_manifest.json"), serde_json::to_string(&manifest).unwrap()).unwrap();

    let ws = WorkspaceIndex::open(&ws_root).expect("open workspace");

    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri_lib = path_to_uri(&lib_str);
    let uri_test = path_to_uri(&test_str);

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri_lib },
            "position": { "line": 0, "character": 7 },
            "newName": "calculate"
        }
    });

    let res = handle_rename(&req, Some(&ws), &overlay_mgr, &mut conv).expect("rename succeeds");
    let changes = res["changes"].as_object().expect("changes object");

    // Must touch both src/lib.rs and tests/test_run.rs
    assert!(changes.contains_key(&uri_lib), "must edit src/lib.rs");
    assert!(changes.contains_key(&uri_test), "must edit tests/test_run.rs");

    let edits_lib = changes[&uri_lib].as_array().expect("edits array for lib");
    let edits_test = changes[&uri_test].as_array().expect("edits array for test");

    // Exactly 2 edits in lib.rs (declaration + reference call)
    assert_eq!(edits_lib.len(), 2, "lib.rs must contain exactly 2 edits (decl and call), no duplicates");
    // Exactly 1 edit in test_run.rs (test call)
    assert_eq!(edits_test.len(), 1, "test_run.rs must contain exactly 1 edit");

    // Total unique edits across workspace must be exactly 3
    let total_edits = edits_lib.len() + edits_test.len();
    assert_eq!(total_edits, 3, "Total unique edits must equal 3");

    // Apply WorkspaceEdit to verify clean result
    let mut files = HashMap::new();
    files.insert(uri_lib.clone(), lib_content.to_string());
    files.insert(uri_test.clone(), test_content.to_string());

    let updated = apply_workspace_edit(&files, &res);
    let updated_lib = &updated[&uri_lib];
    let updated_test = &updated[&uri_test];

    assert!(updated_lib.contains("pub fn calculate(x: i32) -> i32"));
    assert!(updated_lib.contains("calculate(10)"));
    assert!(!updated_lib.contains("compute"));

    assert!(updated_test.contains("calculate(20)"));
    assert!(!updated_test.contains("compute"));
}

#[test]
fn test_rename_overlapping_different_edits_returns_internal_error() {
    let temp_dir = tempfile::tempdir().unwrap();
    let ws_root = temp_dir.path().to_path_buf();
    let pkg_root = ws_root.join("my_pkg");
    fs::create_dir_all(pkg_root.join("src")).unwrap();

    let lib_rs = pkg_root.join("src/lib.rs");
    let lib_str = lib_rs.to_str().unwrap().to_string();
    let lib_content = "pub fn compute(x: i32) -> i32 {\n    x * 2\n}\n";
    fs::write(&lib_rs, lib_content).unwrap();

    let variants_dir = ws_root.join("target/ridx/variants/dev");
    let deps_dir = variants_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();

    let mut writer = IndexWriter::new("my_pkg", "0.1.0");
    writer.set_unit_key("unit_lib");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "compute".to_string(),
        path: "my_pkg::compute".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 15,
    });
    // Inject reference overlapping with declaration on line 1, but with different start_col (9 instead of 8)
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 1,
        start_col: 9,
        end_line: 1,
        end_col: 16,
        target_def_id: Some(1),
        target_path: "my_pkg::compute".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: Some("unit_lib".to_string()),
        target_line: 1,
        target_col: 8,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });

    let lib_idx_path = deps_dir.join("my_pkg_lib.ridx");
    let mut f_lib = fs::File::create(&lib_idx_path).unwrap();
    writer.write_to(&mut f_lib).unwrap();
    drop(f_lib);

    let ws = WorkspaceIndex::open(&lib_idx_path).unwrap();

    let mut overlay_mgr = OverlayManager::new();
    let uri_lib = path_to_uri(&lib_str);
    overlay_mgr.open_document(&uri_lib, &lib_str, 1, lib_content.to_string());
    let mut conv = PositionConverter::new();

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri_lib },
            "position": { "line": 0, "character": 7 },
            "newName": "calculate"
        }
    });

    let res = handle_rename(&req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(res.is_err(), "Overlapping rename edits must return an error");
    let err = res.unwrap_err();
    assert_eq!(err.code, -32603, "Must return JSON-RPC internal error code -32603");
    assert!(err.message.contains("Overlapping rename edits in file:"), "Error message must specify overlapping rename edits");
    // Named with the platform's separators (the fixture joins `src/lib.rs` onto a Windows directory).
    let shown = if cfg!(windows) { lib_str.replace('/', "\\") } else { lib_str.to_string() };
    assert!(err.message.contains(&shown), "Error message must include the file path: {}", err.message);
}
