/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for $/setTrace notification part 2 (Domain 1: Lifecycle & Control).

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
fn test_set_trace_repeated_transitions() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    for val in &["off", "messages", "verbose", "off", "verbose"] {
        let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": val } });
        let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
        assert!(res.unwrap());
        assert_eq!(out, b"");
    }
}

#[test]
fn test_set_trace_numeric_value_tolerated() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": 1 } });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
}

#[test]
fn test_set_trace_boolean_value_tolerated() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": true } });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
}

#[test]
fn test_set_trace_object_value_tolerated() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": { "level": "debug" } } });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
}

#[test]
fn test_set_trace_array_value_tolerated() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": ["verbose"] } });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
}

#[test]
fn test_set_trace_preserves_lifecycle_state() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    assert!(lc.is_initialized());
    let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "verbose" } });
    dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    assert!(lc.is_initialized());
    assert!(!lc.is_shutdown());
}

#[test]
fn test_set_trace_preserves_cancelled_requests() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let cancel = json!({ "jsonrpc": "2.0", "method": "$/cancelRequest", "params": { "id": 777 } });
    dispatch_client(cancel, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    assert_eq!(*can.lock().unwrap(), HashSet::from([json!(777)]));

    let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "off" } });
    dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    assert_eq!(*can.lock().unwrap(), HashSet::from([json!(777)]));
}

#[test]
fn test_set_trace_interleaved_with_definition() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let msg_trace = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "messages" } });
    dispatch_client(msg_trace, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    let msg_def = json!({
        "jsonrpc": "2.0", "id": 888, "method": "textDocument/hover",
        "params": { "textDocument": { "uri": "file:///tmp/dummy.rs" }, "position": { "line": 0, "character": 0 } }
    });
    dispatch_client(msg_def, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    let val = extract_response(&out);
    assert_eq!(val["id"], 888);
}

#[test]
fn test_set_trace_overlay_documents_unaffected() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let uri = "file:///tmp/dummy.rs";
    let open_msg = json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": uri,
                "languageId": "rust",
                "version": 1,
                "text": "fn dummy() {}"
            }
        }
    });
    dispatch_client(open_msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    let file = ridx::lsp::protocol::uri_to_path(uri);
    let before_ov = ov.get(&file).unwrap();
    assert_eq!(before_ov.text, "fn dummy() {}");
    assert_eq!(before_ov.version, 1);

    let trace_msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "verbose" } });
    let res = dispatch_client(trace_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
    assert_eq!(out, b"");
    assert_eq!(ov.get(&file).unwrap().text, "fn dummy() {}");
}

#[test]
fn test_set_trace_no_response_id_leak() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "off" } });
    dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    assert_eq!(out, b"");
}
