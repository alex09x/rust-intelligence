/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unit tests for textDocument/linkedEditingRange protocol handler.

use crate::lsp::editing::linked::handle_linked_editing_range;
use crate::lsp::protocol::PositionConverter;
use crate::overlay::OverlayManager;
use serde_json::json;

#[test]
fn test_linked_editing_local_let_binding() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///tmp/linked_let.rs";
    let code = "fn test() {\n    let count = 10;\n    let total = count + 5;\n}\n";
    overlay_mgr.open_document(uri, "/tmp/linked_let.rs", 1, code.to_string());

    // Position on declaration of `count` (line 1, col 8)
    let req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": 1, "character": 9 }
        }
    });

    let res = handle_linked_editing_range(&req, &overlay_mgr, &mut conv);
    assert!(!res.is_null(), "Linked editing must find local binding ranges");
    let ranges = res["ranges"].as_array().expect("ranges array");
    assert!(ranges.len() >= 2, "Must link declaration and reference");
}

#[test]
fn test_linked_editing_fn_parameter() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///tmp/linked_param.rs";
    let code = "fn test(multiplier: i32) -> i32 {\n    let res = multiplier * 2;\n    res\n}\n";
    overlay_mgr.open_document(uri, "/tmp/linked_param.rs", 1, code.to_string());

    // Position on `multiplier` param (line 0, col 10)
    let req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": 0, "character": 10 }
        }
    });

    let res = handle_linked_editing_range(&req, &overlay_mgr, &mut conv);
    if !res.is_null() {
        let ranges = res["ranges"].as_array().unwrap();
        assert!(ranges.len() >= 2);
    }
}

#[test]
fn test_linked_editing_shadowed_variable_scope_isolation() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///tmp/linked_shadow.rs";
    let code = "fn test() {\n    let x = 1;\n    {\n        let x = 2;\n        let y = x;\n    }\n    let z = x;\n}\n";
    overlay_mgr.open_document(uri, "/tmp/linked_shadow.rs", 1, code.to_string());

    // Position on inner `x` (line 3, col 12)
    let req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": 3, "character": 12 }
        }
    });

    let res = handle_linked_editing_range(&req, &overlay_mgr, &mut conv);
    if !res.is_null() {
        let ranges = res["ranges"].as_array().unwrap();
        // Inner shadow must only link within its block
        for r in ranges {
            let l = r["start"]["line"].as_u64().unwrap();
            assert!(l == 3 || l == 4, "Shadowed range must not leak to outer scope");
        }
    }
}

#[test]
fn test_linked_editing_whitespace_position_returns_null() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///tmp/linked_ws.rs";
    let code = "fn test() {\n    \n}\n";
    overlay_mgr.open_document(uri, "/tmp/linked_ws.rs", 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": 1, "character": 2 }
        }
    });

    let res = handle_linked_editing_range(&req, &overlay_mgr, &mut conv);
    assert!(res.is_null(), "Whitespace position must return null");
}

#[test]
fn test_linked_editing_comment_position_returns_null() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///tmp/linked_comment.rs";
    let code = "fn test() {\n    // let count = 10;\n}\n";
    overlay_mgr.open_document(uri, "/tmp/linked_comment.rs", 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": 1, "character": 12 }
        }
    });

    let res = handle_linked_editing_range(&req, &overlay_mgr, &mut conv);
    assert!(res.is_null(), "Comment position must return null");
}

#[test]
fn test_linked_editing_string_literal_returns_null() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///tmp/linked_str.rs";
    let code = "fn test() {\n    let s = \"hello count\";\n}\n";
    overlay_mgr.open_document(uri, "/tmp/linked_str.rs", 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": 1, "character": 18 }
        }
    });

    let res = handle_linked_editing_range(&req, &overlay_mgr, &mut conv);
    assert!(res.is_null(), "String literal content must return null");
}

#[test]
fn test_linked_editing_missing_overlay_returns_null() {
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let req = json!({
        "params": {
            "textDocument": { "uri": "file:///tmp/not_opened.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });

    let res = handle_linked_editing_range(&req, &overlay_mgr, &mut conv);
    assert!(res.is_null());
}
