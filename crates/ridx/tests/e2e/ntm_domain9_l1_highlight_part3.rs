/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/documentHighlight part 3:
//! local binding highlights, highlight kinds (Write vs Text), and lexical scoping.

use ridx::lsp::navigation::references::handle_document_highlight;
use ridx::lsp::protocol::PositionConverter;
use ridx::overlay::OverlayManager;
use serde_json::json;

fn make_hl_req(uri: &str, line: u32, col: u32) -> serde_json::Value {
    json!({
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": col }
        }
    })
}

#[test]
fn test_document_highlight_local_var_two_usages() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let file = "/tmp/hl_p3_1.rs";
    let uri = "file:///tmp/hl_p3_1.rs";
    let code = "fn run() {\n    let alpha = 1;\n    let _ = alpha + 2;\n}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_hl_req(uri, 1, 8);
    let res = handle_document_highlight(&req, None, &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["range"]["start"]["line"], 1);
    assert_eq!(arr[1]["range"]["start"]["line"], 2);
}

#[test]
fn test_document_highlight_local_var_three_usages() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let file = "/tmp/hl_p3_2.rs";
    let uri = "file:///tmp/hl_p3_2.rs";
    let code = "fn run() {\n    let beta = 10;\n    let _ = beta + 1;\n    let _ = beta + 2;\n}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_hl_req(uri, 1, 8);
    let res = handle_document_highlight(&req, None, &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0]["range"]["start"]["line"], 1);
    assert_eq!(arr[1]["range"]["start"]["line"], 2);
    assert_eq!(arr[2]["range"]["start"]["line"], 3);
}

#[test]
fn test_document_highlight_first_is_write_kind_3() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let file = "/tmp/hl_p3_3.rs";
    let uri = "file:///tmp/hl_p3_3.rs";
    let code = "fn f() {\n    let target = 42;\n    let _ = target;\n}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_hl_req(uri, 1, 8);
    let res = handle_document_highlight(&req, None, &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["kind"], 3);
}

#[test]
fn test_document_highlight_subsequent_are_text_kind_1() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let file = "/tmp/hl_p3_4.rs";
    let uri = "file:///tmp/hl_p3_4.rs";
    let code = "fn f() {\n    let item = 1;\n    let _ = item;\n    let _ = item;\n}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_hl_req(uri, 1, 8);
    let res = handle_document_highlight(&req, None, &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[1]["kind"], 1);
    assert_eq!(arr[2]["kind"], 1);
}

#[test]
fn test_document_highlight_mut_variable_reassignment() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let file = "/tmp/hl_p3_5.rs";
    let uri = "file:///tmp/hl_p3_5.rs";
    let code = "fn f() {\n    let mut count = 0;\n    count = 1;\n    let _ = count;\n}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_hl_req(uri, 1, 12);
    let res = handle_document_highlight(&req, None, &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0]["range"]["start"]["line"], 1);
    assert_eq!(arr[1]["range"]["start"]["line"], 2);
    assert_eq!(arr[2]["range"]["start"]["line"], 3);
}

#[test]
fn test_document_highlight_inner_scope_shadowing_isolated() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let file = "/tmp/hl_p3_6.rs";
    let uri = "file:///tmp/hl_p3_6.rs";
    let code = "fn f() {\n    let x = 1;\n    {\n        let x = 2;\n        let _ = x;\n    }\n}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    // Query on inner `let x = 2;` at line 3, col 12
    let req = make_hl_req(uri, 3, 12);
    let res = handle_document_highlight(&req, None, &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["range"]["start"]["line"], 3);
    assert_eq!(arr[1]["range"]["start"]["line"], 4);
}

#[test]
fn test_document_highlight_outer_scope_ignores_inner_shadow() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let file = "/tmp/hl_p3_7.rs";
    let uri = "file:///tmp/hl_p3_7.rs";
    let code = "fn f() {\n    let x = 1;\n    {\n        let x = 2;\n    }\n    let _ = x;\n}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    // Query on outer `let x = 1;` at line 1, col 8
    let req = make_hl_req(uri, 1, 8);
    let res = handle_document_highlight(&req, None, &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["range"]["start"]["line"], 1);
    assert_eq!(arr[1]["range"]["start"]["line"], 5);
}

#[test]
fn test_document_highlight_function_parameter_highlight() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let file = "/tmp/hl_p3_8.rs";
    let uri = "file:///tmp/hl_p3_8.rs";
    let code = "fn compute(factor: i32) {\n    let _ = factor * 2;\n}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_hl_req(uri, 0, 11);
    let res = handle_document_highlight(&req, None, &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["range"]["start"]["line"], 0);
    assert_eq!(arr[1]["range"]["start"]["line"], 1);
}

#[test]
fn test_document_highlight_sequential_shadowing_first() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let file = "/tmp/hl_p3_9.rs";
    let uri = "file:///tmp/hl_p3_9.rs";
    let code = "fn f() {\n    let item = 1;\n    let _ = item;\n    let item = 2;\n    let _ = item;\n}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    // Query on first `let item = 1;` at line 1, col 8
    let req = make_hl_req(uri, 1, 8);
    let res = handle_document_highlight(&req, None, &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["range"]["start"]["line"], 1);
    assert_eq!(arr[1]["range"]["start"]["line"], 2);
}

#[test]
fn test_document_highlight_sequential_shadowing_second() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let file = "/tmp/hl_p3_10.rs";
    let uri = "file:///tmp/hl_p3_10.rs";
    let code = "fn f() {\n    let item = 1;\n    let _ = item;\n    let item = 2;\n    let _ = item;\n}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    // Query on second `let item = 2;` at line 3, col 8
    let req = make_hl_req(uri, 3, 8);
    let res = handle_document_highlight(&req, None, &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["range"]["start"]["line"], 3);
    assert_eq!(arr[1]["range"]["start"]["line"], 4);
}

#[test]
fn test_document_highlight_fn_two_parameters_distinct() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let file = "/tmp/hl_p3_11.rs";
    let uri = "file:///tmp/hl_p3_11.rs";
    let code = "fn compute(left_arg: i32, right_arg: i32) {\n    let _ = left_arg + right_arg;\n    let _ = left_arg * 2;\n}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    // Query on left_arg at line 0, col 11
    let req = make_hl_req(uri, 0, 11);
    let res = handle_document_highlight(&req, None, &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0]["range"]["start"]["line"], 0);
    assert_eq!(arr[1]["range"]["start"]["line"], 1);
    assert_eq!(arr[2]["range"]["start"]["line"], 2);
}

