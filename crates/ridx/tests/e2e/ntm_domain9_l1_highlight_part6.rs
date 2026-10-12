/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/documentHighlight part 6:
//! comments, string literals, whitespace, scoping isolation, and mutable parameters.

use ridx::lsp::navigation::handle_document_highlight;
use ridx::lsp::protocol::PositionConverter;
use ridx::overlay::OverlayManager;
use serde_json::json;

fn make_pos_req(uri: &str, line: u32, col: u32) -> serde_json::Value {
    json!({
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": col }
        }
    })
}

#[test]
fn test_document_highlight_empty_uri_returns_empty() {
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let req = make_pos_req("", 0, 0);
    let res = handle_document_highlight(&req, None, &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]));
}

#[test]
fn test_document_highlight_whitespace_between_tokens_returns_empty() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let file = "/tmp/hl_p6_ws.rs";
    let uri = "file:///tmp/hl_p6_ws.rs";
    let code = "fn run() {\n    let val = 10;\n    let _ = val;\n}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    // Position (1, 1) is whitespace indentation before `let`
    let req = make_pos_req(uri, 1, 1);
    let res = handle_document_highlight(&req, None, &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]));
}

#[test]
fn test_document_highlight_inside_line_comment_returns_empty() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let file = "/tmp/hl_p6_lc.rs";
    let uri = "file:///tmp/hl_p6_lc.rs";
    let code = "fn run() {\n    // this is a comment mentioning val\n    let val = 5;\n}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    // Position (1, 15) is inside the comment text
    let req = make_pos_req(uri, 1, 15);
    let res = handle_document_highlight(&req, None, &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]));
}

#[test]
fn test_document_highlight_inside_block_comment_returns_empty() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let file = "/tmp/hl_p6_bc.rs";
    let uri = "file:///tmp/hl_p6_bc.rs";
    let code = "fn run() {\n    /* multi-line comment\n       containing token val */\n    let val = 42;\n}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    // Position (2, 10) is inside the multi-line block comment
    let req = make_pos_req(uri, 2, 10);
    let res = handle_document_highlight(&req, None, &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]));
}

#[test]
fn test_document_highlight_inside_string_literal_returns_empty() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let file = "/tmp/hl_p6_str.rs";
    let uri = "file:///tmp/hl_p6_str.rs";
    let code = "fn run() {\n    let text = \"literal value inside quotes\";\n    let _ = text;\n}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    // Position (1, 20) is inside the string literal
    let req = make_pos_req(uri, 1, 20);
    let res = handle_document_highlight(&req, None, &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]));
}

#[test]
fn test_document_highlight_nested_blocks_lexical_scope() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let file = "/tmp/hl_p6_nest.rs";
    let uri = "file:///tmp/hl_p6_nest.rs";
    let code = "fn run() {\n    let val = 1;\n    {\n        {\n            let _ = val;\n        }\n    }\n}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    // Query on outer `let val = 1;` at line 1, col 8
    let req = make_pos_req(uri, 1, 8);
    let res = handle_document_highlight(&req, None, &overlay_mgr, &mut conv);
    let items = res.as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["kind"], 3);
    assert_eq!(items[0]["range"]["start"]["line"], 1);
    assert_eq!(items[1]["kind"], 1);
    assert_eq!(items[1]["range"]["start"]["line"], 4);
}

#[test]
fn test_document_highlight_shadowing_across_sibling_blocks() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let file = "/tmp/hl_p6_sib.rs";
    let uri = "file:///tmp/hl_p6_sib.rs";
    let code = "fn run() {\n    {\n        let sibling = 1;\n        let _ = sibling;\n    }\n    {\n        let sibling = 2;\n        let _ = sibling;\n    }\n}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    // Query on sibling in first block at line 2, col 12
    let req = make_pos_req(uri, 2, 12);
    let res = handle_document_highlight(&req, None, &overlay_mgr, &mut conv);
    let items = res.as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["kind"], 3);
    assert_eq!(items[0]["range"]["start"]["line"], 2);
    assert_eq!(items[1]["kind"], 1);
    assert_eq!(items[1]["range"]["start"]["line"], 3);
}

#[test]
fn test_document_highlight_mut_parameter_highlight() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let file = "/tmp/hl_p6_param.rs";
    let uri = "file:///tmp/hl_p6_param.rs";
    let code = "fn run(mut accumulator: i64) {\n    accumulator += 10;\n    let _ = accumulator;\n}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    // Query on parameter `accumulator` at line 0, col 12
    let req = make_pos_req(uri, 0, 12);
    let res = handle_document_highlight(&req, None, &overlay_mgr, &mut conv);
    let items = res.as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["kind"], 3);
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[1]["kind"], 1);
    assert_eq!(items[1]["range"]["start"]["line"], 1);
    assert_eq!(items[2]["kind"], 1);
    assert_eq!(items[2]["range"]["start"]["line"], 2);
}
