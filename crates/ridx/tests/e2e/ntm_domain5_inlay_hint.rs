/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/inlayHint and inlayHint/resolve (Domain 5: Intelligence & Completion).

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

    let init_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "capabilities": {}
        }
    });
    let _ = dispatch_lsp_message(LspMessage::Client(init_req), &mut out, &mut lc, &mut mr, &mut ov, &can, None);
    let init_notif = json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {}
    });
    let _ = dispatch_lsp_message(LspMessage::Client(init_notif), &mut out, &mut lc, &mut mr, &mut ov, &can, None);
    (mr, ov, lc, can)
}

fn open_test_file(ov: &mut OverlayManager, uri: &str, version: i32, text: &str) {
    let notif = json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": uri,
                "languageId": "rust",
                "version": version,
                "text": text
            }
        }
    });
    let mut mr = MultiRootWorkspace::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();
    let _ = dispatch_lsp_message(LspMessage::Client(notif), &mut out, &mut lc, &mut mr, ov, &can, None);
}

fn send_inlay_hint_req(
    params: serde_json::Value,
    mr: &mut MultiRootWorkspace,
    ov: &mut OverlayManager,
    lc: &mut LifecycleManager,
    can: &Arc<Mutex<HashSet<serde_json::Value>>>,
) -> serde_json::Value {
    let req = json!({
        "jsonrpc": "2.0",
        "id": 50,
        "method": "textDocument/inlayHint",
        "params": params
    });
    let mut out = Vec::new();
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None);
    extract_response(&out)
}

#[test]
fn test_inlay_hint_missing_params_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 51,
        "method": "textDocument/inlayHint"
    });
    let mut out = Vec::new();
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, &mut lc, &mut mr, &mut ov, &can, None);
    let res = extract_response(&out);
    assert_eq!(res["error"]["code"], -32602);
}

#[test]
fn test_inlay_hint_missing_text_document_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "range": {
            "start": { "line": 0, "character": 0 },
            "end": { "line": 10, "character": 0 }
        }
    });
    let res = send_inlay_hint_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], -32602);
}

#[test]
fn test_inlay_hint_missing_range_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "textDocument": { "uri": "file:///tmp/ih_missing_range.rs" }
    });
    let res = send_inlay_hint_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], -32602);
}

#[test]
fn test_inlay_hint_invalid_range_negative_line_rejected() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "textDocument": { "uri": "file:///tmp/ih_neg_line.rs" },
        "range": {
            "start": { "line": -1, "character": 0 },
            "end": { "line": 5, "character": 0 }
        }
    });
    let res = send_inlay_hint_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], -32602);
}

#[test]
fn test_inlay_hint_empty_document_returns_empty_array() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/ih_empty.rs";
    open_test_file(&mut ov, uri, 1, "");

    let params = json!({
        "textDocument": { "uri": uri },
        "range": {
            "start": { "line": 0, "character": 0 },
            "end": { "line": 0, "character": 0 }
        }
    });
    let res = send_inlay_hint_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"], json!([]));
}

#[test]
fn test_inlay_hint_nonexistent_file_returns_empty_array() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "textDocument": { "uri": "file:///tmp/nonexistent_inlay_hints.rs" },
        "range": {
            "start": { "line": 0, "character": 0 },
            "end": { "line": 5, "character": 0 }
        }
    });
    let res = send_inlay_hint_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"], json!([]));
}

#[test]
fn test_inlay_hint_no_let_bindings_returns_empty_array() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/ih_no_lets.rs";
    open_test_file(&mut ov, uri, 1, "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "range": {
            "start": { "line": 0, "character": 0 },
            "end": { "line": 2, "character": 1 }
        }
    });
    let res = send_inlay_hint_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"], json!([]));
}

#[test]
fn test_inlay_hint_explicit_type_annotation_skipped() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/ih_explicit_type.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let explicit_val: u64 = 42;\n}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "range": {
            "start": { "line": 0, "character": 0 },
            "end": { "line": 2, "character": 1 }
        }
    });
    let res = send_inlay_hint_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"], json!([]));
}

#[test]
fn test_inlay_hint_closed_document_cleared_from_overlay() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/ih_close.rs";
    open_test_file(&mut ov, uri, 1, "fn main() { let x = 1; }\n");

    let close_notif = json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didClose",
        "params": {
            "textDocument": { "uri": uri }
        }
    });
    let mut out = Vec::new();
    let _ = dispatch_lsp_message(LspMessage::Client(close_notif), &mut out, &mut lc, &mut mr, &mut ov, &can, None);

    let params = json!({
        "textDocument": { "uri": uri },
        "range": {
            "start": { "line": 0, "character": 0 },
            "end": { "line": 0, "character": 20 }
        }
    });
    let res = send_inlay_hint_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"], json!([]));
}

#[test]
fn test_inlay_hint_dirty_overlay_reflected() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/ih_dirty.rs";
    open_test_file(&mut ov, uri, 1, "fn first() {}\n");

    let change_notif = json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": uri, "version": 2 },
            "contentChanges": [{ "text": "fn modified() {\n    let explicit_k: bool = true;\n}\n" }]
        }
    });
    let mut out = Vec::new();
    let _ = dispatch_lsp_message(LspMessage::Client(change_notif), &mut out, &mut lc, &mut mr, &mut ov, &can, None);

    let params = json!({
        "textDocument": { "uri": uri },
        "range": {
            "start": { "line": 0, "character": 0 },
            "end": { "line": 2, "character": 1 }
        }
    });
    let res = send_inlay_hint_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"], json!([]));
}
