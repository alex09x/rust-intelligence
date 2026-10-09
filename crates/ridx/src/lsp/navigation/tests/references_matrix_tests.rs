/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unit tests for LSP textDocument/references and textDocument/documentHighlight.

use super::setup_test_navigation;
use crate::lsp::navigation::references::{handle_document_highlight, handle_references};
use crate::overlay::OverlayManager;
use serde_json::json;
use std::fs;

#[test]
fn test_refs_include_declaration_flag_true() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 8 },
            "context": { "includeDeclaration": true }
        }
    });
    let res = handle_references(&req, Some(&ws), &overlay_mgr, &mut conv);
    let arr = res.as_array().expect("Array result");
    assert!(!arr.is_empty());
    // At least one entry points to declaration line 0
    assert!(arr.iter().any(|r| r["range"]["start"]["line"] == 0));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_refs_include_declaration_flag_false() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 8 },
            "context": { "includeDeclaration": false }
        }
    });
    let res = handle_references(&req, Some(&ws), &overlay_mgr, &mut conv);
    let arr = res.as_array().expect("Array result");
    // Declaration line 0 is excluded, only call sites remain
    assert!(arr.iter().all(|r| r["range"]["start"]["line"] != 0));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_refs_local_binding_references_in_overlay() {
    let (temp_dir, ws, mut conv) = (
        setup_test_navigation().0,
        setup_test_navigation().1,
        setup_test_navigation().3,
    );
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    let mut ov_mgr = OverlayManager::new();
    let code = "fn test() {\n    let my_counter = 42;\n    let sum = my_counter + 1;\n    println!(\"{}\", my_counter);\n}\n";
    ov_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 9 }
        }
    });
    let res = handle_references(&req, Some(&ws), &ov_mgr, &mut conv);
    let arr = res.as_array().expect("Array result");
    assert_eq!(arr.len(), 3, "Must find declaration and both read usages");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_refs_doc_highlight_read_and_write() {
    let (temp_dir, ws, mut conv) = (
        setup_test_navigation().0,
        setup_test_navigation().1,
        setup_test_navigation().3,
    );
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    let mut ov_mgr = OverlayManager::new();
    let code = "fn test() {\n    let mut val = 1;\n    val = 2;\n    let _ = val;\n}\n";
    ov_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 13 }
        }
    });
    let hl = handle_document_highlight(&req, Some(&ws), &ov_mgr, &mut conv);
    let arr = hl.as_array().expect("Highlight array");
    assert_eq!(arr.len(), 3);
    // Write access kind is 3, read is 1
    assert!(arr.iter().any(|h| h["kind"] == 3));
    assert!(arr.iter().any(|h| h["kind"] == 1));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_refs_empty_position_returns_empty_array() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 0 }
        }
    });
    assert_eq!(
        handle_references(&req, Some(&ws), &overlay_mgr, &mut conv),
        json!([])
    );
    assert_eq!(
        handle_document_highlight(&req, Some(&ws), &overlay_mgr, &mut conv),
        json!([])
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_refs_missing_workspace_returns_empty_array() {
    let (temp_dir, _, overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 8 }
        }
    });
    assert_eq!(
        handle_references(&req, None, &overlay_mgr, &mut conv),
        json!([])
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_refs_nonexistent_uri_returns_empty_array() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_navigation();
    let req = json!({
        "params": {
            "textDocument": { "uri": "file:///path/nonexistent.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });
    assert_eq!(
        handle_references(&req, Some(&ws), &overlay_mgr, &mut conv),
        json!([])
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_refs_out_of_bounds_returns_empty_array() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 9999, "character": 9999 }
        }
    });
    assert_eq!(
        handle_references(&req, Some(&ws), &overlay_mgr, &mut conv),
        json!([])
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_refs_zero_end_span_handling() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 9, "character": 8 },
            "context": { "includeDeclaration": true }
        }
    });
    let res = handle_references(&req, Some(&ws), &overlay_mgr, &mut conv);
    let arr = res.as_array().expect("Array result");
    assert!(!arr.is_empty());

    let _ = fs::remove_dir_all(&temp_dir);
}
