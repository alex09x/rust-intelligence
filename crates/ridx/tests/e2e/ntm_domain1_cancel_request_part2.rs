/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for $/cancelRequest notification part 2 (Domain 1: Lifecycle & Control).

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

    let init = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    dispatch_client(init, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    (mr, ov, lc, can)
}

fn dispatch_client<W: std::io::Write>(
    msg: serde_json::Value,
    out: &mut W,
    mr: &mut MultiRootWorkspace,
    ov: &mut OverlayManager,
    lc: &mut LifecycleManager,
    can: &Arc<Mutex<HashSet<serde_json::Value>>>,
) -> std::io::Result<bool> {
    dispatch_lsp_message(LspMessage::Client(msg), out, lc, mr, ov, can, None)
}

fn send_cancel(
    id: serde_json::Value,
    mr: &mut MultiRootWorkspace,
    ov: &mut OverlayManager,
    lc: &mut LifecycleManager,
    can: &Arc<Mutex<HashSet<serde_json::Value>>>,
) {
    let mut out = Vec::new();
    let msg = json!({
        "jsonrpc": "2.0",
        "method": "$/cancelRequest",
        "params": { "id": id.clone() }
    });
    dispatch_client(msg, &mut out, mr, ov, lc, can).unwrap();
    assert_eq!(out, b"");
}

#[test]
fn test_cancel_request_cancels_definition() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();
    send_cancel(json!(101), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(*can.lock().unwrap(), HashSet::from([json!(101)]));

    let msg = json!({
        "jsonrpc": "2.0", "id": 101, "method": "textDocument/definition",
        "params": { "textDocument": { "uri": "file:///tmp/dummy.rs" }, "position": { "line": 0, "character": 0 } }
    });
    dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    let val = extract_response(&out);
    assert_eq!(val["id"], 101);
    assert_eq!(val["error"]["code"], -32800);
    assert_eq!(val["error"]["message"], "Request cancelled");
    assert_eq!(*can.lock().unwrap(), HashSet::new());
}

#[test]
fn test_cancel_request_cancels_references() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();
    send_cancel(json!(102), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(*can.lock().unwrap(), HashSet::from([json!(102)]));

    let msg = json!({
        "jsonrpc": "2.0", "id": 102, "method": "textDocument/references",
        "params": { "textDocument": { "uri": "file:///tmp/dummy.rs" }, "position": { "line": 0, "character": 0 }, "context": { "includeDeclaration": true } }
    });
    dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    let val = extract_response(&out);
    assert_eq!(val["id"], 102);
    assert_eq!(val["error"]["code"], -32800);
    assert_eq!(val["error"]["message"], "Request cancelled");
    assert_eq!(*can.lock().unwrap(), HashSet::new());
}

#[test]
fn test_cancel_request_cancels_completion() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();
    send_cancel(json!(103), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(*can.lock().unwrap(), HashSet::from([json!(103)]));

    let msg = json!({
        "jsonrpc": "2.0", "id": 103, "method": "textDocument/completion",
        "params": { "textDocument": { "uri": "file:///tmp/dummy.rs" }, "position": { "line": 0, "character": 0 } }
    });
    dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    let val = extract_response(&out);
    assert_eq!(val["id"], 103);
    assert_eq!(val["error"]["code"], -32800);
    assert_eq!(val["error"]["message"], "Request cancelled");
    assert_eq!(*can.lock().unwrap(), HashSet::new());
}

#[test]
fn test_cancel_request_cancels_hover() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();
    send_cancel(json!(104), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(*can.lock().unwrap(), HashSet::from([json!(104)]));

    let msg = json!({
        "jsonrpc": "2.0", "id": 104, "method": "textDocument/hover",
        "params": { "textDocument": { "uri": "file:///tmp/dummy.rs" }, "position": { "line": 0, "character": 0 } }
    });
    dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    let val = extract_response(&out);
    assert_eq!(val["id"], 104);
    assert_eq!(val["error"]["code"], -32800);
    assert_eq!(val["error"]["message"], "Request cancelled");
    assert_eq!(*can.lock().unwrap(), HashSet::new());
}

#[test]
fn test_cancel_request_cancels_rename() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();
    send_cancel(json!(105), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(*can.lock().unwrap(), HashSet::from([json!(105)]));

    let msg = json!({
        "jsonrpc": "2.0", "id": 105, "method": "textDocument/rename",
        "params": { "textDocument": { "uri": "file:///tmp/dummy.rs" }, "position": { "line": 0, "character": 0 }, "newName": "xyz" }
    });
    dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    let val = extract_response(&out);
    assert_eq!(val["id"], 105);
    assert_eq!(val["error"]["code"], -32800);
    assert_eq!(val["error"]["message"], "Request cancelled");
    assert_eq!(*can.lock().unwrap(), HashSet::new());
}

#[test]
fn test_cancel_request_cancels_formatting() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();
    send_cancel(json!(106), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(*can.lock().unwrap(), HashSet::from([json!(106)]));

    let msg = json!({
        "jsonrpc": "2.0", "id": 106, "method": "textDocument/formatting",
        "params": { "textDocument": { "uri": "file:///tmp/dummy.rs" }, "options": { "tabSize": 4, "insertSpaces": true } }
    });
    dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    let val = extract_response(&out);
    assert_eq!(val["id"], 106);
    assert_eq!(val["error"]["code"], -32800);
    assert_eq!(val["error"]["message"], "Request cancelled");
    assert_eq!(*can.lock().unwrap(), HashSet::new());
}

#[test]
fn test_cancel_request_cancels_code_action() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();
    send_cancel(json!(107), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(*can.lock().unwrap(), HashSet::from([json!(107)]));

    let msg = json!({
        "jsonrpc": "2.0", "id": 107, "method": "textDocument/codeAction",
        "params": { "textDocument": { "uri": "file:///tmp/dummy.rs" }, "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } }, "context": { "diagnostics": [] } }
    });
    dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    let val = extract_response(&out);
    assert_eq!(val["id"], 107);
    assert_eq!(val["error"]["code"], -32800);
    assert_eq!(val["error"]["message"], "Request cancelled");
    assert_eq!(*can.lock().unwrap(), HashSet::new());
}

#[test]
fn test_cancel_request_concurrent_multiple_ids() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    for cid in [201, 202, 203] {
        send_cancel(json!(cid), &mut mr, &mut ov, &mut lc, &can);
    }
    assert_eq!(*can.lock().unwrap(), HashSet::from([json!(201), json!(202), json!(203)]));

    let msg = json!({
        "jsonrpc": "2.0", "id": 202, "method": "textDocument/hover",
        "params": { "textDocument": { "uri": "file:///tmp/dummy.rs" }, "position": { "line": 0, "character": 0 } }
    });
    dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    let val = extract_response(&out);
    assert_eq!(val["id"], 202);
    assert_eq!(val["error"]["code"], -32800);
    assert_eq!(val["error"]["message"], "Request cancelled");

    let guard = can.lock().unwrap();
    assert_eq!(*guard, HashSet::from([json!(201), json!(203)]));
}

#[test]
fn test_cancel_request_preserves_unrelated_requests() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    send_cancel(json!(301), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(*can.lock().unwrap(), HashSet::from([json!(301)]));

    let msg = json!({
        "jsonrpc": "2.0", "id": 302, "method": "textDocument/hover",
        "params": { "textDocument": { "uri": "file:///tmp/dummy.rs" }, "position": { "line": 0, "character": 0 } }
    });
    dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    let val = extract_response(&out);
    assert_eq!(val["id"], 302);
    assert_eq!(val.get("error"), None);
    assert_eq!(*can.lock().unwrap(), HashSet::from([json!(301)]));
}

#[test]
fn test_cancel_request_preserves_exact_request_id() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    send_cancel(json!("req-uuid-401"), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(*can.lock().unwrap(), HashSet::from([json!("req-uuid-401")]));

    let msg = json!({
        "jsonrpc": "2.0", "id": "req-uuid-401", "method": "textDocument/hover",
        "params": { "textDocument": { "uri": "file:///tmp/dummy.rs" }, "position": { "line": 0, "character": 0 } }
    });
    dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    let val = extract_response(&out);
    assert_eq!(val["id"], "req-uuid-401");
    assert_eq!(val["error"]["code"], -32800);
    assert_eq!(val["error"]["message"], "Request cancelled");
    assert_eq!(*can.lock().unwrap(), HashSet::new());
}
