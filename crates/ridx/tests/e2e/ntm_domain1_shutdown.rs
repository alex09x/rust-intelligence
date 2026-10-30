/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for shutdown request (Domain 1: Lifecycle & Control).

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::{LifecycleManager, ServerState};
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
fn test_shutdown_returns_null() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let msg = json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown", "params": null });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());

    let val = extract_response(&out);
    assert_eq!(val["id"], 2);
    assert_eq!(val["result"], serde_json::Value::Null);
}

#[test]
fn test_shutdown_transitions_state_to_shutdown() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    assert_eq!(lc.state(), ServerState::Initialized);
    assert!(!lc.is_shutdown());

    let mut out = Vec::new();
    let msg = json!({ "jsonrpc": "2.0", "id": 3, "method": "shutdown" });
    dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    assert_eq!(lc.state(), ServerState::ShutDown);
    assert!(lc.is_shutdown());
}

#[test]
fn test_shutdown_duplicate_rejected_code_32600() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let msg1 = json!({ "jsonrpc": "2.0", "id": 4, "method": "shutdown" });
    dispatch_client(msg1, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    out.clear();
    let msg2 = json!({ "jsonrpc": "2.0", "id": 5, "method": "shutdown" });
    let _ = dispatch_client(msg2, &mut out, &mut mr, &mut ov, &mut lc, &can);

    let val = extract_response(&out);
    assert_eq!(val["id"], 5);
    assert_eq!(val["error"]["code"], -32600);
}

#[test]
fn test_shutdown_rejects_definition_code_32600() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let shut = json!({ "jsonrpc": "2.0", "id": 6, "method": "shutdown" });
    dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    out.clear();
    let def = json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "textDocument/definition",
        "params": {
            "textDocument": { "uri": "file:///tmp/dummy.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });
    let _ = dispatch_client(def, &mut out, &mut mr, &mut ov, &mut lc, &can);

    let val = extract_response(&out);
    assert_eq!(val["id"], 7);
    assert_eq!(val["error"]["code"], -32600);
}

#[test]
fn test_shutdown_rejects_references_code_32600() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let shut = json!({ "jsonrpc": "2.0", "id": 8, "method": "shutdown" });
    dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    out.clear();
    let refs = json!({
        "jsonrpc": "2.0",
        "id": 9,
        "method": "textDocument/references",
        "params": {
            "textDocument": { "uri": "file:///tmp/dummy.rs" },
            "position": { "line": 0, "character": 0 },
            "context": { "includeDeclaration": true }
        }
    });
    let _ = dispatch_client(refs, &mut out, &mut mr, &mut ov, &mut lc, &can);

    let val = extract_response(&out);
    assert_eq!(val["id"], 9);
    assert_eq!(val["error"]["code"], -32600);
}

#[test]
fn test_shutdown_rejects_type_definition_code_32600() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let shut = json!({ "jsonrpc": "2.0", "id": 10, "method": "shutdown" });
    dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    out.clear();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 11,
        "method": "textDocument/typeDefinition",
        "params": {
            "textDocument": { "uri": "file:///tmp/dummy.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });
    let _ = dispatch_client(req, &mut out, &mut mr, &mut ov, &mut lc, &can);

    let val = extract_response(&out);
    assert_eq!(val["id"], 11);
    assert_eq!(val["error"]["code"], -32600);
}

#[test]
fn test_shutdown_rejects_implementation_code_32600() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let shut = json!({ "jsonrpc": "2.0", "id": 12, "method": "shutdown" });
    dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    out.clear();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 13,
        "method": "textDocument/implementation",
        "params": {
            "textDocument": { "uri": "file:///tmp/dummy.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });
    let _ = dispatch_client(req, &mut out, &mut mr, &mut ov, &mut lc, &can);

    let val = extract_response(&out);
    assert_eq!(val["id"], 13);
    assert_eq!(val["error"]["code"], -32600);
}

#[test]
fn test_shutdown_rejects_hover_code_32600() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let shut = json!({ "jsonrpc": "2.0", "id": 14, "method": "shutdown" });
    dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    out.clear();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 15,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": "file:///tmp/dummy.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });
    let _ = dispatch_client(req, &mut out, &mut mr, &mut ov, &mut lc, &can);

    let val = extract_response(&out);
    assert_eq!(val["id"], 15);
    assert_eq!(val["error"]["code"], -32600);
}

#[test]
fn test_shutdown_rejects_completion_code_32600() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let shut = json!({ "jsonrpc": "2.0", "id": 16, "method": "shutdown" });
    dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    out.clear();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 17,
        "method": "textDocument/completion",
        "params": {
            "textDocument": { "uri": "file:///tmp/dummy.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });
    let _ = dispatch_client(req, &mut out, &mut mr, &mut ov, &mut lc, &can);

    let val = extract_response(&out);
    assert_eq!(val["id"], 17);
    assert_eq!(val["error"]["code"], -32600);
}

#[test]
fn test_shutdown_rejects_document_symbol_code_32600() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let shut = json!({ "jsonrpc": "2.0", "id": 18, "method": "shutdown" });
    dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    out.clear();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 19,
        "method": "textDocument/documentSymbol",
        "params": {
            "textDocument": { "uri": "file:///tmp/dummy.rs" }
        }
    });
    let _ = dispatch_client(req, &mut out, &mut mr, &mut ov, &mut lc, &can);

    let val = extract_response(&out);
    assert_eq!(val["id"], 19);
    assert_eq!(val["error"]["code"], -32600);
}
