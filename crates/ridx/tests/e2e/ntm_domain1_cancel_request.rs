/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for $/cancelRequest notification (Domain 1: Lifecycle & Control).

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

#[test]
fn test_cancel_request_integer_id_stored() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "method": "$/cancelRequest",
        "params": { "id": 42 }
    });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
    assert_eq!(out, b"");
    assert_eq!(*can.lock().unwrap(), HashSet::from([json!(42)]));
}

#[test]
fn test_cancel_request_string_id_stored() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "method": "$/cancelRequest",
        "params": { "id": "req-xyz-123" }
    });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
    assert_eq!(out, b"");
    assert_eq!(*can.lock().unwrap(), HashSet::from([json!("req-xyz-123")]));
}

#[test]
fn test_cancel_request_returns_ok_true() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "method": "$/cancelRequest",
        "params": { "id": 100 }
    });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
    assert_eq!(out, b"");
    assert_eq!(*can.lock().unwrap(), HashSet::from([json!(100)]));
}

#[test]
fn test_cancel_request_produces_no_stdout() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "method": "$/cancelRequest",
        "params": { "id": 101 }
    });
    dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    assert_eq!(out, b"");
}

#[test]
fn test_cancel_request_triggers_code_32800() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    // 1. Cancel id 50
    let cancel_msg = json!({
        "jsonrpc": "2.0",
        "method": "$/cancelRequest",
        "params": { "id": 50 }
    });
    dispatch_client(cancel_msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    // 2. Dispatch request with id 50
    out.clear();
    let def_msg = json!({
        "jsonrpc": "2.0",
        "id": 50,
        "method": "textDocument/definition",
        "params": {
            "textDocument": { "uri": "file:///tmp/dummy.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });
    let res = dispatch_client(def_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());

    let val = extract_response(&out);
    assert_eq!(val["id"], 50);
    assert_eq!(val["error"]["code"], -32800);
    assert_eq!(val["error"]["message"], "Request cancelled");
}

#[test]
fn test_cancel_request_removes_id_upon_cancel() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let cancel = json!({ "jsonrpc": "2.0", "method": "$/cancelRequest", "params": { "id": 60 } });
    dispatch_client(cancel, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    assert_eq!(*can.lock().unwrap(), HashSet::from([json!(60)]));

    let def_msg = json!({
        "jsonrpc": "2.0",
        "id": 60,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": "file:///tmp/dummy.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });
    dispatch_client(def_msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    // Removed from cancelled set once cancelled response is sent
    assert_eq!(*can.lock().unwrap(), HashSet::new());
}

#[test]
fn test_cancel_request_nonexistent_id_noop() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let def_msg = json!({
        "jsonrpc": "2.0",
        "id": 70,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": "file:///tmp/dummy.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });
    dispatch_client(def_msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    let val = extract_response(&out);
    assert_eq!(val["id"], 70);
    // Not cancelled, returns standard result without error
    assert_eq!(val.get("error"), None);
}

#[test]
fn test_cancel_request_duplicate_cancel_id() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let cancel1 = json!({ "jsonrpc": "2.0", "method": "$/cancelRequest", "params": { "id": 80 } });
    let cancel2 = json!({ "jsonrpc": "2.0", "method": "$/cancelRequest", "params": { "id": 80 } });
    dispatch_client(cancel1, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    dispatch_client(cancel2, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    assert_eq!(*can.lock().unwrap(), HashSet::from([json!(80)]));
}

#[test]
fn test_cancel_request_missing_id_tolerated() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let msg = json!({ "jsonrpc": "2.0", "method": "$/cancelRequest", "params": {} });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
    assert_eq!(*can.lock().unwrap(), HashSet::new());
}

#[test]
fn test_cancel_request_missing_params_tolerated() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let msg = json!({ "jsonrpc": "2.0", "method": "$/cancelRequest" });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
    assert_eq!(*can.lock().unwrap(), HashSet::new());
}
