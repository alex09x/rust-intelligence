/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/implementation part 7:
//! LSP dispatch, request ID preservation, and boundary edge cases.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::navigation::handle_implementation;
use ridx::lsp::protocol::{LspMessage, PositionConverter};
use ridx::lsp::workspace::MultiRootWorkspace;
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

#[test]
fn test_implementation_preserves_numeric_request_id() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 9001,
        "method": "textDocument/implementation",
        "params": {
            "textDocument": { "uri": "file:///tmp/unregistered.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, &mut lc, &mut mr, &mut ov, &can, None);
    let resp = extract_response(&out);
    assert_eq!(resp["id"], 9001);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_implementation_preserves_string_request_id() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": "req-impl-str-1",
        "method": "textDocument/implementation",
        "params": {
            "textDocument": { "uri": "file:///tmp/unregistered.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, &mut lc, &mut mr, &mut ov, &can, None);
    let resp = extract_response(&out);
    assert_eq!(resp["id"], "req-impl-str-1");
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_implementation_preserves_large_numeric_id() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 88888888,
        "method": "textDocument/implementation",
        "params": {
            "textDocument": { "uri": "file:///tmp/unregistered.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, &mut lc, &mut mr, &mut ov, &can, None);
    let resp = extract_response(&out);
    assert_eq!(resp["id"], 88888888);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_implementation_whitespace_cursor_returns_empty_array() {
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let req = make_pos_req("file:///tmp/empty.rs", 0, 0);
    let res = handle_implementation(&req, None, &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]));
}

#[test]
fn test_implementation_unregistered_uri_returns_empty_array() {
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let req = make_pos_req("file:///unregistered/file.rs", 10, 5);
    let res = handle_implementation(&req, None, &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]));
}

#[test]
fn test_implementation_out_of_bounds_line_returns_empty_array() {
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let req = make_pos_req("file:///tmp/some.rs", 99999, 0);
    let res = handle_implementation(&req, None, &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]));
}

#[test]
fn test_implementation_out_of_bounds_col_returns_empty_array() {
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let req = make_pos_req("file:///tmp/some.rs", 0, 99999);
    let res = handle_implementation(&req, None, &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]));
}

#[test]
fn test_implementation_no_workspace_returns_empty_array() {
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let req = make_pos_req("file:///tmp/no_ws.rs", 1, 1);
    let res = handle_implementation(&req, None, &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]));
}
