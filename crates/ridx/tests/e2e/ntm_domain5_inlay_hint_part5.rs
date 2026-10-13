/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for inlayHint/resolve data fields and inlayHint buffers (Domain 5: Intelligence & Completion).

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

fn setup_initialized_server() -> (
    MultiRootWorkspace,
    OverlayManager,
    LifecycleManager,
    Arc<Mutex<HashSet<serde_json::Value>>>,
) {
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
        "id": 53,
        "method": "textDocument/inlayHint",
        "params": params
    });
    let mut out = Vec::new();
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None);
    extract_response(&out)
}

fn send_inlay_hint_resolve_req(
    params: serde_json::Value,
    mr: &mut MultiRootWorkspace,
    ov: &mut OverlayManager,
    lc: &mut LifecycleManager,
    can: &Arc<Mutex<HashSet<serde_json::Value>>>,
) -> serde_json::Value {
    let req = json!({
        "jsonrpc": "2.0",
        "id": 63,
        "method": "inlayHint/resolve",
        "params": params
    });
    let mut out = Vec::new();
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None);
    extract_response(&out)
}

#[test]
fn test_inlay_hint_resolve_preserves_numeric_data_field() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "position": { "line": 0, "character": 1 },
        "label": ": u8",
        "data": 42
    });
    let res = send_inlay_hint_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["data"], 42);
}

#[test]
fn test_inlay_hint_resolve_preserves_boolean_data_field() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "position": { "line": 1, "character": 0 },
        "label": ": ()",
        "data": true
    });
    let res = send_inlay_hint_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["data"], true);
}

#[test]
fn test_inlay_hint_resolve_preserves_null_data_field() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "position": { "line": 2, "character": 3 },
        "label": ": i16",
        "data": null
    });
    let res = send_inlay_hint_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["data"], serde_json::Value::Null);
}

#[test]
fn test_inlay_hint_utf8_multibyte_characters_in_buffer() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/ih_utf8.rs";
    open_test_file(&mut ov, uri, 1, "// Rocket: 🚀\nfn greet() {}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "range": {
            "start": { "line": 0, "character": 0 },
            "end": { "line": 1, "character": 10 }
        }
    });
    let res = send_inlay_hint_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"], json!([]));
}

#[test]
fn test_inlay_hint_cr_lf_line_endings_in_buffer() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/ih_crlf.rs";
    open_test_file(&mut ov, uri, 1, "fn win_line() {\r\n}\r\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "range": {
            "start": { "line": 0, "character": 0 },
            "end": { "line": 1, "character": 1 }
        }
    });
    let res = send_inlay_hint_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"], json!([]));
}

#[test]
fn test_inlay_hint_zero_length_file_returns_empty() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/ih_zero_len.rs";
    open_test_file(&mut ov, uri, 1, "");

    let params = json!({
        "textDocument": { "uri": uri },
        "range": {
            "start": { "line": 0, "character": 0 },
            "end": { "line": 10, "character": 0 }
        }
    });
    let res = send_inlay_hint_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"], json!([]));
}

#[test]
fn test_inlay_hint_let_inside_nested_block() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/ih_nested.rs";
    open_test_file(&mut ov, uri, 1, "fn outer() {\n    {\n        let val_annotated: usize = 1;\n    }\n}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "range": {
            "start": { "line": 0, "character": 0 },
            "end": { "line": 4, "character": 1 }
        }
    });
    let res = send_inlay_hint_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"], json!([]));
}

#[test]
fn test_inlay_hint_let_mut_binding_without_type() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/ih_let_mut.rs";
    open_test_file(&mut ov, uri, 1, "fn runner() {\n    let mut counter_val: u32 = 0;\n}\n");

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
fn test_inlay_hint_response_is_json_array() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/ih_array_type.rs";
    open_test_file(&mut ov, uri, 1, "fn nop() {}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "range": {
            "start": { "line": 0, "character": 0 },
            "end": { "line": 0, "character": 11 }
        }
    });
    let res = send_inlay_hint_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"], json!([]));
}
