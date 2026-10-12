/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for $/cancelRequest notification part 4 (Domain 1: Lifecycle & Control).

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
    dispatch_lsp_message(LspMessage::Client(init), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
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

fn assert_cancelled_rejection(method: &str, req_id: i64, params: serde_json::Value) {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    // 1. Cancel the request id
    let cancel = json!({ "jsonrpc": "2.0", "method": "$/cancelRequest", "params": { "id": req_id } });
    let _ = dispatch_client(cancel, &mut out, &mut mr, &mut ov, &mut lc, &can);
    out.clear();

    // 2. Dispatch request with that id
    let req = json!({
        "jsonrpc": "2.0",
        "id": req_id,
        "method": method,
        "params": params
    });
    let _ = dispatch_client(req, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    assert_eq!(val["id"], req_id);
    assert_eq!(val["error"]["code"], -32800); // RequestCancelled
}

#[test]
fn test_cancel_request_multiple_different_ids_accumulated() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let c1 = json!({ "jsonrpc": "2.0", "method": "$/cancelRequest", "params": { "id": 1001 } });
    let c2 = json!({ "jsonrpc": "2.0", "method": "$/cancelRequest", "params": { "id": 1002 } });
    let c3 = json!({ "jsonrpc": "2.0", "method": "$/cancelRequest", "params": { "id": 1003 } });

    assert!(dispatch_client(c1, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap());
    assert!(dispatch_client(c2, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap());
    assert!(dispatch_client(c3, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap());

    let lock = can.lock().unwrap();
    assert_eq!(*lock, HashSet::from([json!(1001), json!(1002), json!(1003)]));
}

#[test]
fn test_cancel_request_uuid_string_id() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let uuid_str = "c2e742a0-48e2-4bd5-a1f9-83bc9a6e345b";
    let cancel = json!({ "jsonrpc": "2.0", "method": "$/cancelRequest", "params": { "id": uuid_str } });
    assert!(dispatch_client(cancel, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap());
    assert_eq!(*can.lock().unwrap(), HashSet::from([json!(uuid_str)]));
}

#[test]
fn test_cancel_request_rejects_definition_with_32800() {
    assert_cancelled_rejection(
        "textDocument/definition",
        201,
        json!({
            "textDocument": { "uri": "file:///workspace/lib.rs" },
            "position": { "line": 0, "character": 0 }
        }),
    );
}

#[test]
fn test_cancel_request_rejects_completion_with_32800() {
    assert_cancelled_rejection(
        "textDocument/completion",
        202,
        json!({
            "textDocument": { "uri": "file:///workspace/lib.rs" },
            "position": { "line": 0, "character": 0 }
        }),
    );
}

#[test]
fn test_cancel_request_rejects_hover_with_32800() {
    assert_cancelled_rejection(
        "textDocument/hover",
        203,
        json!({
            "textDocument": { "uri": "file:///workspace/lib.rs" },
            "position": { "line": 0, "character": 0 }
        }),
    );
}

#[test]
fn test_cancel_request_rejects_references_with_32800() {
    assert_cancelled_rejection(
        "textDocument/references",
        204,
        json!({
            "textDocument": { "uri": "file:///workspace/lib.rs" },
            "position": { "line": 0, "character": 0 }
        }),
    );
}

#[test]
fn test_cancel_request_rejects_formatting_with_32800() {
    assert_cancelled_rejection(
        "textDocument/formatting",
        205,
        json!({
            "textDocument": { "uri": "file:///workspace/lib.rs" },
            "options": { "tabSize": 4, "insertSpaces": true }
        }),
    );
}

#[test]
fn test_cancel_request_rejects_rename_with_32800() {
    assert_cancelled_rejection(
        "textDocument/rename",
        206,
        json!({
            "textDocument": { "uri": "file:///workspace/lib.rs" },
            "position": { "line": 0, "character": 0 },
            "newName": "renamed"
        }),
    );
}

#[test]
fn test_cancel_request_rejects_document_symbol_with_32800() {
    assert_cancelled_rejection(
        "textDocument/documentSymbol",
        207,
        json!({
            "textDocument": { "uri": "file:///workspace/lib.rs" }
        }),
    );
}

#[test]
fn test_cancel_request_rejects_workspace_symbol_with_32800() {
    assert_cancelled_rejection(
        "workspace/symbol",
        208,
        json!({
            "query": "search"
        }),
    );
}
