/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/inlineValue pattern destructuring and lifecycle (Domain 5: Intelligence & Completion).

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::LspMessage;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use serde_json::json;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

fn extract_response(out: &[u8]) -> serde_json::Value {
    let s = String::from_utf8_lossy(out);
    for part in s.split("Content-Length: ") {
        if let Some(idx) = part.find("\r\n\r\n") {
            let body = &part[idx + 4..];
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(body.trim()) {
                return v;
            }
        }
    }
    serde_json::Value::Null
}

fn setup_initialized_server() -> (MultiRootWorkspace, OverlayManager, LifecycleManager, Arc<Mutex<HashSet<serde_json::Value>>>) {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let init = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    dispatch_lsp_message(LspMessage::Client(init), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
    (mr, ov, lc, can)
}

fn send_inline_val_req(
    uri: &str,
    sl: u32,
    el: u32,
    mr: &mut MultiRootWorkspace,
    ov: &mut OverlayManager,
    lc: &mut LifecycleManager,
    can: &Arc<Mutex<HashSet<serde_json::Value>>>,
) -> serde_json::Value {
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 70,
        "method": "textDocument/inlineValue",
        "params": {
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": sl, "character": 0 },
                "end": { "line": el, "character": 100 }
            }
        }
    });
    dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None).unwrap();
    extract_response(&out)
}

fn open_test_file(ov: &mut OverlayManager, uri: &str, version: i32, text: &str) {
    let file = ridx::lsp::protocol::uri_to_path(uri);
    ov.open_or_update(uri, &file, version, text.to_string());
}

#[test]
fn test_inline_value_missing_params_defaults_gracefully() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 71,
        "method": "textDocument/inlineValue"
    });
    dispatch_lsp_message(LspMessage::Client(req), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
    let res = extract_response(&out);
    assert_eq!(res["result"], json!([]));
}

#[test]
fn test_inline_value_missing_range_defaults_to_line_zero() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/iv_no_range.rs";
    open_test_file(&mut ov, uri, 1, "target_var + 1\nsecond_line\n");

    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 72,
        "method": "textDocument/inlineValue",
        "params": {
            "textDocument": { "uri": uri }
        }
    });
    dispatch_lsp_message(LspMessage::Client(req), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
    let res = extract_response(&out);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["variableName"], "target_var");
}

#[test]
fn test_inline_value_missing_text_document_defaults_empty() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 73,
        "method": "textDocument/inlineValue",
        "params": {
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 0, "character": 10 }
            }
        }
    });
    dispatch_lsp_message(LspMessage::Client(req), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
    let res = extract_response(&out);
    assert_eq!(res["result"], json!([]));
}

#[test]
fn test_inline_value_cancelled_request_returns_cancelled_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let cancel_id = json!(74);
    can.lock().unwrap().insert(cancel_id.clone());

    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": cancel_id,
        "method": "textDocument/inlineValue",
        "params": {
            "textDocument": { "uri": "file:///tmp/iv_cancel.rs" },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 1, "character": 0 }
            }
        }
    });
    dispatch_lsp_message(LspMessage::Client(req), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
    let res = extract_response(&out);
    assert_eq!(res["error"]["code"], -32800);
}

#[test]
fn test_inline_value_match_arm_pattern_identifiers() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/iv_match.rs";
    open_test_file(&mut ov, uri, 1, "match opt {\n    Some(found_val) => found_val,\n    None => 0,\n}\n");

    let res = send_inline_val_req(uri, 1, 1, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["variableName"], "Some");
    assert_eq!(items[1]["variableName"], "found_val");
    assert_eq!(items[2]["variableName"], "found_val");
}

#[test]
fn test_inline_value_tuple_destructuring_identifiers() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/iv_tuple.rs";
    open_test_file(&mut ov, uri, 1, "fn run() {\n    let (item_a, item_b) = pair;\n}\n");

    let res = send_inline_val_req(uri, 1, 1, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["variableName"], "item_a");
    assert_eq!(items[1]["variableName"], "item_b");
    assert_eq!(items[2]["variableName"], "pair");
}

#[test]
fn test_inline_value_struct_destructuring_identifiers() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/iv_struct.rs";
    open_test_file(&mut ov, uri, 1, "fn run() {\n    let Point { coord_x, coord_y } = pt;\n}\n");

    let res = send_inline_val_req(uri, 1, 1, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 4);
    assert_eq!(items[0]["variableName"], "Point");
    assert_eq!(items[1]["variableName"], "coord_x");
    assert_eq!(items[2]["variableName"], "coord_y");
    assert_eq!(items[3]["variableName"], "pt");
}

#[test]
fn test_inline_value_closure_parameter_identifiers() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/iv_closure.rs";
    open_test_file(&mut ov, uri, 1, "fn run() {\n    let calc = |factor_x, factor_y| factor_x * factor_y;\n}\n");

    let res = send_inline_val_req(uri, 1, 1, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 5);
    assert_eq!(items[0]["variableName"], "calc");
    assert_eq!(items[1]["variableName"], "factor_x");
    assert_eq!(items[2]["variableName"], "factor_y");
    assert_eq!(items[3]["variableName"], "factor_x");
    assert_eq!(items[4]["variableName"], "factor_y");
}

#[test]
fn test_inline_value_closed_document_cleared_from_overlay() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/iv_close.rs";
    open_test_file(&mut ov, uri, 1, "let transient_var = 1;\n");

    let file = ridx::lsp::protocol::uri_to_path(uri);
    ov.close(&file);

    let res = send_inline_val_req(uri, 0, 0, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"], json!([]));
}

#[test]
fn test_inline_value_dirty_overlay_reflected() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/iv_dirty.rs";
    open_test_file(&mut ov, uri, 1, "let before_edit = 1;\n");
    open_test_file(&mut ov, uri, 2, "let after_edit = 2;\n");

    let res = send_inline_val_req(uri, 0, 0, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["variableName"], "after_edit");
}
