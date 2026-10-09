/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unit tests for LSP textDocument/typeDefinition.

use super::setup_test_navigation;
use crate::lsp::navigation::handle_type_definition;
use serde_json::json;
use std::fs;

#[test]
fn test_typedef_from_struct_type() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    // Cursor on `Bar` on line 2 (0-based 1), char 12
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 12 }
        }
    });
    let res = handle_type_definition(&req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(!res.is_null());
    assert_eq!(res["range"]["start"]["line"], 1);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_typedef_dirty_overlay_safely_returns_null() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());
    let code = fs::read_to_string(&file_path).unwrap();

    overlay_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, code.clone());
    // Insert single comment line at the top
    overlay_mgr.update(
        file_path.to_str().unwrap(),
        2,
        format!("// header\n{}", code),
    );

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 12 }
        }
    });
    let res = handle_type_definition(&req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(res.is_null());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_typedef_dirty_overlay_unmapped_returns_null() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    overlay_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, "a\nb\n".to_string());
    overlay_mgr.update(file_path.to_str().unwrap(), 2, "xyz\n".to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 4, "character": 0 }
        }
    });
    assert!(handle_type_definition(&req, Some(&ws), &overlay_mgr, &mut conv).is_null());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_typedef_dirty_overlay_modified_imports_returns_null() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());
    let code = fs::read_to_string(&file_path).unwrap();

    overlay_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, code.clone());
    overlay_mgr.update(
        file_path.to_str().unwrap(),
        2,
        format!("use std::sync::Arc;\n{}", code),
    );

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 12 }
        }
    });
    assert!(handle_type_definition(&req, Some(&ws), &overlay_mgr, &mut conv).is_null());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_typedef_missing_workspace_returns_null() {
    let (temp_dir, _, overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 12 }
        }
    });
    assert!(handle_type_definition(&req, None, &overlay_mgr, &mut conv).is_null());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_typedef_unknown_position_returns_null() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 0 }
        }
    });
    assert!(handle_type_definition(&req, Some(&ws), &overlay_mgr, &mut conv).is_null());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_typedef_clean_overlay_returns_def() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());
    let code = fs::read_to_string(&file_path).unwrap();

    overlay_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, code);

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 12 }
        }
    });
    let res = handle_type_definition(&req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(!res.is_null());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_typedef_nonexistent_uri_returns_null() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_navigation();
    let req = json!({
        "params": {
            "textDocument": { "uri": "file:///path/does/not/exist.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });
    assert!(handle_type_definition(&req, Some(&ws), &overlay_mgr, &mut conv).is_null());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_typedef_out_of_bounds_character_clamped() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 9999 }
        }
    });
    let res = handle_type_definition(&req, Some(&ws), &overlay_mgr, &mut conv);
    // Large character offset does not panic and returns null or closest type
    let _ = res;

    let _ = fs::remove_dir_all(&temp_dir);
}
