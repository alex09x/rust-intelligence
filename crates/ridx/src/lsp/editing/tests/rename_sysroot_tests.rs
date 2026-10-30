/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Tests for rename operations on workspace files containing `sysroot` in path
//! versus external sysroot symbols.

use crate::lsp::editing::rename::handle_rename;
use crate::lsp::protocol::{path_to_uri, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::writer::{IndexWriter, RawDefinition, RawReference};
use ridx_format::{DefKind, WorkspaceIndex};
use serde_json::json;
use std::fs::{self, File};

#[test]
fn test_rename_workspace_symbol_with_sysroot_in_path() {
    let temp_dir = tempfile::tempdir().unwrap();
    let ws_root = temp_dir.path().to_path_buf();
    let pkg_root = ws_root.join("crates/x");
    let sysroot_dir = pkg_root.join("src/sysroot");
    fs::create_dir_all(&sysroot_dir).unwrap();

    let file_path = sysroot_dir.join("cas.rs");
    let file_str = file_path.to_str().unwrap().to_string();
    let content = "pub fn store() -> i32 {\n    42\n}\n\npub fn caller() -> i32 {\n    store()\n}\n";
    fs::write(&file_path, content).unwrap();

    let variants_dir = ws_root.join("target/ridx/variants/dev");
    let deps_dir = variants_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();

    let mut writer = IndexWriter::new("x", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "store".to_string(),
        path: "x::store".to_string(),
        signature: Some("fn() -> i32".to_string()),
        file: file_str.clone(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 13,
    });
    writer.add_reference(RawReference {
        file: file_str.clone(),
        start_line: 6,
        start_col: 5,
        end_line: 6,
        end_col: 10,
        target_def_id: Some(1),
        target_path: "x::store".to_string(),
        target_file: Some(file_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 8,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });

    let idx_path = variants_dir.join("x.ridx");
    let mut f = File::create(&idx_path).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let meta = json!({
        "name": "x",
        "version": "0.1.0",
        "source_root": pkg_root.to_str().unwrap(),
        "target_kind": "lib",
        "is_dependency": false
    });
    fs::write(deps_dir.join("x.json"), serde_json::to_string(&meta).unwrap()).unwrap();

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

    let mut overlay_mgr = OverlayManager::new();
    let uri = path_to_uri(&file_str);
    overlay_mgr.open_document(&uri, &file_str, 1, content.to_string());
    let mut conv = PositionConverter::new();

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 7 },
            "newName": "persist"
        }
    });

    let res = handle_rename(&req, Some(&ws), &overlay_mgr, &mut conv);
    let edits = res.expect("Rename should succeed for workspace file containing /sysroot/ in path");
    let changes = edits["changes"][&uri].as_array().expect("Changes for file must exist");
    assert_eq!(changes.len(), 2, "Must produce exactly 2 edits (def and ref)");
}

#[test]
fn test_rename_external_sysroot_symbol_rejected() {
    let temp_dir = tempfile::tempdir().unwrap();
    let ws_root = temp_dir.path().to_path_buf();
    let src_dir = ws_root.join("src");
    fs::create_dir_all(&src_dir).unwrap();

    let main_rs = src_dir.join("main.rs");
    let main_str = main_rs.to_str().unwrap().to_string();
    let main_content = "fn main() {\n    let v = alloc::vec::Vec::new();\n}\n";
    fs::write(&main_rs, main_content).unwrap();

    let variants_dir = ws_root.join("target/ridx/variants/dev");
    let deps_dir = variants_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();

    let app_idx_path = variants_dir.join("app.ridx");
    let mut app_writer = IndexWriter::new("app", "0.1.0");
    app_writer.set_unit_key("unit_app");
    app_writer.add_reference(RawReference {
        file: main_str.clone(),
        start_line: 2,
        start_col: 25,
        end_line: 2,
        end_col: 28,
        target_def_id: None,
        target_path: "alloc::vec::Vec".to_string(),
        target_file: Some("library/alloc/src/vec/mod.rs".to_string()),
        target_unit: None,
        target_line: 10,
        target_col: 1,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });
    let mut f_app = File::create(&app_idx_path).unwrap();
    app_writer.write_to(&mut f_app).unwrap();
    drop(f_app);

    let app_meta = json!({
        "name": "app",
        "version": "0.1.0",
        "unit_key": "unit_app",
        "source_root": ws_root.to_str().unwrap(),
        "target_kind": "bin",
        "is_dependency": false
    });
    fs::write(deps_dir.join("app.json"), serde_json::to_string(&app_meta).unwrap()).unwrap();

    let manifest = json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "dev",
        "variant_dir": "variants/dev",
        "workspace_root": ws_root.to_str().unwrap()
    });
    fs::write(ws_root.join("target/ridx/ridx_manifest.json"), serde_json::to_string(&manifest).unwrap()).unwrap();

    let mut ws = WorkspaceIndex::open(&ws_root).expect("open workspace");

    // Create and attach sysroot shard
    let alloc_idx_path = temp_dir.path().join("alloc.ridx");
    let mut alloc_writer = IndexWriter::new("alloc", "1.80.0");
    alloc_writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "Vec".to_string(),
        path: "alloc::vec::Vec".to_string(),
        signature: None,
        file: "library/alloc/src/vec/mod.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 10,
        end_col: 15,
    });
    let mut f_alloc = File::create(&alloc_idx_path).unwrap();
    alloc_writer.write_to(&mut f_alloc).unwrap();
    drop(f_alloc);

    ws.attach_shard_paths(&[alloc_idx_path]);

    let mut overlay_mgr = OverlayManager::new();
    let uri = path_to_uri(&main_str);
    overlay_mgr.open_document(&uri, &main_str, 1, main_content.to_string());
    let mut conv = PositionConverter::new();

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 24 },
            "newName": "MyVec"
        }
    });

    let res = handle_rename(&req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(res.is_err(), "Rename on external sysroot symbol must be rejected");
    let err = res.unwrap_err();
    assert_eq!(err.code, -32803, "Must return LSP error code -32803 (RequestFailed)");
    assert_eq!(err.message, "Cannot rename external symbol 'Vec'", "Error message must specify Cannot rename external symbol 'Vec'");
}
