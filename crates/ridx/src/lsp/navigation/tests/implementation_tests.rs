/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unit tests for LSP textDocument/implementation.

use super::setup_test_navigation;
use crate::lsp::navigation::handle_implementation;
use crate::overlay::OverlayManager;
use serde_json::json;
use std::fs;

#[test]
fn test_impl_trait_finds_implementors() {
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
    let res = handle_implementation(&req, Some(&ws), &overlay_mgr, &mut conv);
    let arr = res.as_array().expect("Result must be array");
    assert!(!arr.is_empty(), "Must find at least one implementation of Baz");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_impl_empty_position_returns_empty_array() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 0 }
        }
    });
    let res = handle_implementation(&req, Some(&ws), &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_impl_missing_workspace_returns_empty_array() {
    let (temp_dir, _, overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 11 }
        }
    });
    let res = handle_implementation(&req, None, &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_impl_dirty_overlay_modified_imports_returns_empty_array() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());
    let code = fs::read_to_string(&file_path).unwrap();

    overlay_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, code.clone());
    overlay_mgr.update(file_path.to_str().unwrap(), 2, format!("use std::fmt::Debug;\n{}", code));

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 3, "character": 11 }
        }
    });
    let res = handle_implementation(&req, Some(&ws), &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_impl_target_dirty_overlay_safely_returns_empty() {
    let (temp_dir, ws, _, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());
    let target_code = fs::read_to_string(&file_path).unwrap();

    let mut ov_target = OverlayManager::new();
    ov_target.open_document(&uri, file_path.to_str().unwrap(), 1, target_code.clone());
    ov_target.update(file_path.to_str().unwrap(), 2, format!("// prepended line\n{}", target_code));

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 3, "character": 11 }
        }
    });
    let res = handle_implementation(&req, Some(&ws), &ov_target, &mut conv);
    assert_eq!(res, json!([]));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_impl_function_symbol_returns_empty_array() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    // Cursor on `foo()` function definition which has no trait implementations
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 8 }
        }
    });
    let res = handle_implementation(&req, Some(&ws), &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_impl_nonexistent_uri_returns_empty_array() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_navigation();
    let req = json!({
        "params": {
            "textDocument": { "uri": "file:///path/to/missing.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });
    let res = handle_implementation(&req, Some(&ws), &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_impl_out_of_bounds_returns_empty_array() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 9999, "character": 9999 }
        }
    });
    let res = handle_implementation(&req, Some(&ws), &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]));

    let _ = fs::remove_dir_all(&temp_dir);
}
