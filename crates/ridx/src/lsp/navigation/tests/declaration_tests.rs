/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unit tests for LSP textDocument/declaration.

use super::setup_test_navigation;
use crate::lsp::navigation::{handle_declaration, handle_definition};
use serde_json::json;
use std::fs;

#[test]
fn test_decl_delegates_to_definition() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 4, "character": 19 }
        }
    });
    let decl_res = handle_declaration(&req, Some(&ws), &overlay_mgr, &mut conv);
    let def_res = handle_definition(&req, Some(&ws), &overlay_mgr, &mut conv);
    assert_eq!(decl_res, def_res);
    assert!(!decl_res.is_null());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_decl_empty_position_returns_null() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 0 }
        }
    });
    assert!(handle_declaration(&req, Some(&ws), &overlay_mgr, &mut conv).is_null());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_decl_missing_workspace_returns_null() {
    let (temp_dir, _, overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 4, "character": 19 }
        }
    });
    assert!(handle_declaration(&req, None, &overlay_mgr, &mut conv).is_null());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_decl_overlay_clean() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());
    let code = fs::read_to_string(&file_path).unwrap();

    overlay_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, code);

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 4, "character": 19 }
        }
    });
    let res = handle_declaration(&req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(!res.is_null());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_decl_overlay_dirty_imports_returns_null() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());
    let code = fs::read_to_string(&file_path).unwrap();

    overlay_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, code.clone());
    overlay_mgr.update(file_path.to_str().unwrap(), 2, format!("use std::io;\n{}", code));

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 9, "character": 16 }
        }
    });
    assert!(handle_declaration(&req, Some(&ws), &overlay_mgr, &mut conv).is_null());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_decl_struct_target() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    // Cursor on `Bar` in `pub struct Bar;` on line 2 (0-based 1), char 12
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 12 }
        }
    });
    let res = handle_declaration(&req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(!res.is_null());
    assert_eq!(res["range"]["start"]["line"], 1);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_decl_trait_target() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    // Cursor on `Baz` in `pub trait Baz {}` on line 3 (0-based 2), char 11
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 11 }
        }
    });
    let res = handle_declaration(&req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(!res.is_null());
    assert_eq!(res["range"]["start"]["line"], 2);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_decl_out_of_bounds_returns_null() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 9999, "character": 9999 }
        }
    });
    assert!(handle_declaration(&req, Some(&ws), &overlay_mgr, &mut conv).is_null());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_decl_external_symbol_reference() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    // Cursor on call to `ext_fn()` on line 9 (0-based 8), char 16
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 8, "character": 16 }
        }
    });
    let res = handle_declaration(&req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(!res.is_null());

    let _ = fs::remove_dir_all(&temp_dir);
}
