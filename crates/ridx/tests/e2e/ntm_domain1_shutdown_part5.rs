/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for shutdown request part 5 (Domain 1: Lifecycle & Control).

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
fn test_shutdown_success_response_result_null() {
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
    let _ = dispatch_client(init, &mut out, &mut mr, &mut ov, &mut lc, &can);
    out.clear();

    let shut = json!({ "jsonrpc": "2.0", "id": 501, "method": "shutdown" });
    let _ = dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    assert_eq!(val["id"], 501);
    assert_eq!(val["result"], serde_json::Value::Null);
}

#[test]
fn test_shutdown_string_id_preserved() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let init = json!({
        "jsonrpc": "2.0",
        "id": "init-id",
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    let _ = dispatch_client(init, &mut out, &mut mr, &mut ov, &mut lc, &can);
    out.clear();

    let shut = json!({ "jsonrpc": "2.0", "id": "shut-req-id", "method": "shutdown" });
    let _ = dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    assert_eq!(val["id"], "shut-req-id");
}

#[test]
fn test_shutdown_numeric_large_id_preserved() {
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
    let _ = dispatch_client(init, &mut out, &mut mr, &mut ov, &mut lc, &can);
    out.clear();

    let shut = json!({ "jsonrpc": "2.0", "id": 999_888_777, "method": "shutdown" });
    let _ = dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    assert_eq!(val["id"], 999_888_777);
}

#[test]
fn test_shutdown_duplicate_shutdown_rejected() {
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
    let _ = dispatch_client(init, &mut out, &mut mr, &mut ov, &mut lc, &can);
    out.clear();

    let shut1 = json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" });
    let _ = dispatch_client(shut1, &mut out, &mut mr, &mut ov, &mut lc, &can);
    out.clear();

    let shut2 = json!({ "jsonrpc": "2.0", "id": 3, "method": "shutdown" });
    let _ = dispatch_client(shut2, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    assert_eq!(val["id"], 3);
    assert_eq!(val["error"]["code"], -32600);
}

#[test]
fn test_shutdown_uninitialized_server_shutdown_rejected() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new(); // uninitialized
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let shut = json!({ "jsonrpc": "2.0", "id": 1, "method": "shutdown" });
    let _ = dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    assert_eq!(val["id"], 1);
    assert_eq!(val["error"]["code"], -32002);
}

#[test]
fn test_shutdown_followed_by_did_open_ignored_safely() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } });
    let _ = dispatch_client(init, &mut out, &mut mr, &mut ov, &mut lc, &can);
    out.clear();

    let shut = json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" });
    let _ = dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can);
    out.clear();

    let did_open = json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": { "uri": "file:///workspace/lib.rs", "languageId": "rust", "version": 1, "text": "" }
        }
    });
    let res = dispatch_client(did_open, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
}

#[test]
fn test_shutdown_followed_by_did_change_ignored_safely() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } });
    let _ = dispatch_client(init, &mut out, &mut mr, &mut ov, &mut lc, &can);
    out.clear();

    let shut = json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" });
    let _ = dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can);
    out.clear();

    let did_change = json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": "file:///workspace/lib.rs", "version": 2 },
            "contentChanges": [{ "text": "fn test() {}" }]
        }
    });
    let res = dispatch_client(did_change, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
}

#[test]
fn test_shutdown_followed_by_did_close_ignored_safely() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } });
    let _ = dispatch_client(init, &mut out, &mut mr, &mut ov, &mut lc, &can);
    out.clear();

    let shut = json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" });
    let _ = dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can);
    out.clear();

    let did_close = json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didClose",
        "params": {
            "textDocument": { "uri": "file:///workspace/lib.rs" }
        }
    });
    let res = dispatch_client(did_close, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
}

#[test]
fn test_shutdown_followed_by_set_trace_ignored_safely() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } });
    let _ = dispatch_client(init, &mut out, &mut mr, &mut ov, &mut lc, &can);
    out.clear();

    let shut = json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" });
    let _ = dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can);
    out.clear();

    let set_tr = json!({
        "jsonrpc": "2.0",
        "method": "$/setTrace",
        "params": { "value": "off" }
    });
    let res = dispatch_client(set_tr, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
}

#[test]
fn test_shutdown_jsonrpc_version_in_response() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } });
    let _ = dispatch_client(init, &mut out, &mut mr, &mut ov, &mut lc, &can);
    out.clear();

    let shut = json!({ "jsonrpc": "2.0", "id": 333, "method": "shutdown" });
    let _ = dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    assert_eq!(val["jsonrpc"], "2.0");
    assert_eq!(val["id"], 333);
}
