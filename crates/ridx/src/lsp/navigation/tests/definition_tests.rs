/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unit tests for LSP textDocument/definition.

use super::setup_test_navigation;
use crate::lsp::navigation::handle_definition;
use crate::overlay::OverlayManager;
use serde_json::json;
use std::fs;

#[test]
fn test_def_workspace_compiled_exact_match() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    // Call site of `foo()` in `caller()` is at line 5 (0-based 4), char 19
    let def_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 4, "character": 19 }
        }
    });
    let def_res = handle_definition(&def_req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(!def_res.is_null());
    assert_eq!(def_res["range"]["start"]["line"], 0);
    assert_eq!(def_res["range"]["start"]["character"], 7);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_def_overlay_clean_unmodified() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());
    let code = fs::read_to_string(&file_path).unwrap();

    overlay_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, code);

    let def_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 4, "character": 19 }
        }
    });
    let def_res = handle_definition(&def_req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(!def_res.is_null());
    assert_eq!(def_res["range"]["start"]["line"], 0);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_def_overlay_dirty_mapped_line() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());
    let code = fs::read_to_string(&file_path).unwrap();

    overlay_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, code.clone());
    // Prepend two lines of comments without changing imports
    let updated = format!("// comment 1\n// comment 2\n{}", code);
    overlay_mgr.update(file_path.to_str().unwrap(), 2, updated);

    // Call site is now shifted down by 2 lines: from line 4 to line 6
    let def_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 6, "character": 19 }
        }
    });
    let def_res = handle_definition(&def_req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(!def_res.is_null());
    // Definition target is also shifted by 2 lines in dirty overlay
    assert_eq!(def_res["range"]["start"]["line"], 2);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_def_overlay_dirty_unmapped_returns_null() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    overlay_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, "line1\nline2\n".to_string());
    overlay_mgr.update(file_path.to_str().unwrap(), 2, "new text\n".to_string());

    let def_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 5, "character": 0 }
        }
    });
    let def_res = handle_definition(&def_req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(def_res.is_null());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_def_overlay_dirty_modified_imports_aborts() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());
    let code = fs::read_to_string(&file_path).unwrap();

    overlay_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, code.clone());
    overlay_mgr.update(file_path.to_str().unwrap(), 2, format!("use std::collections::HashMap;\n{}", code));

    let def_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 9, "character": 16 }
        }
    });
    let def_res = handle_definition(&def_req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(def_res.is_null());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_def_overlay_target_file_dirty_line_shift() {
    let (temp_dir, ws, _, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());
    let target_code = fs::read_to_string(&file_path).unwrap();

    let mut ov_target_dirty = OverlayManager::new();
    ov_target_dirty.open_document(&uri, file_path.to_str().unwrap(), 1, target_code.clone());
    ov_target_dirty.update(file_path.to_str().unwrap(), 2, format!("// prepended line\n{}", target_code));
    if let Some(ov) = ov_target_dirty.get_mut(file_path.to_str().unwrap()) {
        ov.symbols.clear(); // fallback to workspace definition, mapped through dirty target
    }

    let def_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 5, "character": 19 }
        }
    });
    let def_res = handle_definition(&def_req, Some(&ws), &ov_target_dirty, &mut conv);
    assert!(!def_res.is_null());
    assert_eq!(def_res["range"]["start"]["line"], 1);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_def_overlay_local_definition_symbol() {
    let (temp_dir, ws, _, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    let mut ov_local = OverlayManager::new();
    ov_local.open_document(&uri, file_path.to_str().unwrap(), 1, "pub fn local_item() {}\n".to_string());

    let def_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 8 }
        }
    });
    let def_res = handle_definition(&def_req, Some(&ws), &ov_local, &mut conv);
    assert!(!def_res.is_null());
    assert_eq!(def_res["range"]["start"]["line"], 0);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_def_empty_position_returns_null() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    let def_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 0 }
        }
    });
    let def_res = handle_definition(&def_req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(def_res.is_null());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_def_missing_workspace_returns_null() {
    let (temp_dir, _, overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    let def_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 4, "character": 19 }
        }
    });
    let def_res = handle_definition(&def_req, None, &overlay_mgr, &mut conv);
    assert!(def_res.is_null());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_def_nonexistent_uri_returns_null() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_navigation();
    let def_req = json!({
        "params": {
            "textDocument": { "uri": "file:///path/to/missing/file.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });
    let def_res = handle_definition(&def_req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(def_res.is_null());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_def_zero_end_span_fallback() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    // `zero_end` has end_line = 0, end_col = 0 in index; test fallback handling
    let def_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 9, "character": 8 }
        }
    });
    let def_res = handle_definition(&def_req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(!def_res.is_null());
    assert_eq!(def_res["range"]["start"]["line"], 9);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_def_file_backed_mod_jumps_to_backing_file_root() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_def_mod_file_{}", std::process::id()));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let lib_file = temp_dir.join("src/lib.rs");
    let sub_file = temp_dir.join("src/sub.rs");
    fs::write(&lib_file, "pub mod sub;\n").unwrap();
    fs::write(&sub_file, "pub fn helper() {}\n").unwrap();

    let mut writer = ridx_format::IndexWriter::new("mod_pkg", "0.1.0");
    writer.add_definition(ridx_format::RawDefinition {
        def_id: 1,
        kind: ridx_format::DefKind::Mod,
        visibility: 0,
        is_macro_expansion: false,
        file: lib_file.to_str().unwrap().to_string(),
        name: "sub".to_string(),
        start_line: 1,
        start_col: 0,
        end_line: 1,
        end_col: 12,
        path: "mod_pkg::sub".to_string(),
        signature: None,
    });
    let ridx_file = temp_dir.join("index.ridx");
    writer.write_to(&mut fs::File::create(&ridx_file).unwrap()).unwrap();

    let mut ws = ridx_format::WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let overlay_mgr = OverlayManager::new();
    let mut conv = crate::lsp::protocol::PositionConverter::new();
    let uri = format!("file://{}", lib_file.to_str().unwrap());

    let def_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 8 }
        }
    });
    let def_res = handle_definition(&def_req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(!def_res.is_null());
    assert_eq!(def_res["uri"].as_str().unwrap(), crate::lsp::uri_windows::raw_file_uri(&sub_file));
    assert_eq!(def_res["range"]["start"]["line"], 0);
    assert_eq!(def_res["range"]["start"]["character"], 0);

    let _ = fs::remove_dir_all(&temp_dir);
}
