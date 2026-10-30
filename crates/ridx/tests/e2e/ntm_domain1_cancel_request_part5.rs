/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for $/cancelRequest notification part 5 (Domain 1: Lifecycle & Control).

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

    let cancel = json!({ "jsonrpc": "2.0", "method": "$/cancelRequest", "params": { "id": req_id } });
    let _ = dispatch_client(cancel, &mut out, &mut mr, &mut ov, &mut lc, &can);
    out.clear();

    let req = json!({
        "jsonrpc": "2.0",
        "id": req_id,
        "method": method,
        "params": params
    });
    let _ = dispatch_client(req, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    assert_eq!(val["id"], req_id);
    assert_eq!(val["error"]["code"], -32800);
}

#[test]
fn test_cancel_request_rejects_type_definition_with_32800() {
    assert_cancelled_rejection(
        "textDocument/typeDefinition",
        301,
        json!({
            "textDocument": { "uri": "file:///workspace/lib.rs" },
            "position": { "line": 0, "character": 0 }
        }),
    );
}

#[test]
fn test_cancel_request_rejects_implementation_with_32800() {
    assert_cancelled_rejection(
        "textDocument/implementation",
        302,
        json!({
            "textDocument": { "uri": "file:///workspace/lib.rs" },
            "position": { "line": 0, "character": 0 }
        }),
    );
}

#[test]
fn test_cancel_request_rejects_document_highlight_with_32800() {
    assert_cancelled_rejection(
        "textDocument/documentHighlight",
        303,
        json!({
            "textDocument": { "uri": "file:///workspace/lib.rs" },
            "position": { "line": 0, "character": 0 }
        }),
    );
}

#[test]
fn test_cancel_request_rejects_code_action_with_32800() {
    assert_cancelled_rejection(
        "textDocument/codeAction",
        304,
        json!({
            "textDocument": { "uri": "file:///workspace/lib.rs" },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 5 } },
            "context": { "diagnostics": [] }
        }),
    );
}

#[test]
fn test_cancel_request_rejects_inlay_hint_with_32800() {
    assert_cancelled_rejection(
        "textDocument/inlayHint",
        305,
        json!({
            "textDocument": { "uri": "file:///workspace/lib.rs" },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 5, "character": 0 } }
        }),
    );
}

#[test]
fn test_cancel_request_rejects_semantic_tokens_with_32800() {
    assert_cancelled_rejection(
        "textDocument/semanticTokens/full",
        306,
        json!({
            "textDocument": { "uri": "file:///workspace/lib.rs" }
        }),
    );
}

#[test]
fn test_cancel_request_removes_id_from_set_after_rejection() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let req_id = 307;
    let cancel = json!({ "jsonrpc": "2.0", "method": "$/cancelRequest", "params": { "id": req_id } });
    let _ = dispatch_client(cancel, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(*can.lock().unwrap(), HashSet::from([json!(req_id)]));
    out.clear();

    let req = json!({
        "jsonrpc": "2.0",
        "id": req_id,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": "file:///workspace/lib.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });
    let _ = dispatch_client(req, &mut out, &mut mr, &mut ov, &mut lc, &can);
    // After handling the cancellation, id is removed from cancelled set
    assert_eq!(*can.lock().unwrap(), HashSet::new());
}

#[test]
fn test_cancel_request_subsequent_request_with_same_id_not_cancelled() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let req_id = 308;
    // 1. Cancel and reject first request
    let cancel = json!({ "jsonrpc": "2.0", "method": "$/cancelRequest", "params": { "id": req_id } });
    let _ = dispatch_client(cancel, &mut out, &mut mr, &mut ov, &mut lc, &can);
    out.clear();

    let req1 = json!({
        "jsonrpc": "2.0",
        "id": req_id,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": "file:///workspace/lib.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });
    let _ = dispatch_client(req1, &mut out, &mut mr, &mut ov, &mut lc, &can);
    out.clear();

    // 2. Second request with same id now gets normal response (null or value), NOT an error!
    let req2 = json!({
        "jsonrpc": "2.0",
        "id": req_id,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": "file:///workspace/lib.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });
    let _ = dispatch_client(req2, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    assert_eq!(val["id"], req_id);
    assert_eq!(val["result"], serde_json::Value::Null);
}

#[test]
fn test_cancel_request_does_not_affect_overlay_contents() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    ov.open_or_update("file:///lib.rs", "/lib.rs", 1, "pub fn stable() {}".to_string());
    let cancel = json!({ "jsonrpc": "2.0", "method": "$/cancelRequest", "params": { "id": 999 } });
    let _ = dispatch_client(cancel, &mut out, &mut mr, &mut ov, &mut lc, &can);

    let doc = ov.get("/lib.rs").unwrap();
    assert_eq!(doc.text, "pub fn stable() {}");
}

#[test]
fn test_cancel_request_stress_ten_consecutive_cancels() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    for i in 500..510 {
        let cancel = json!({ "jsonrpc": "2.0", "method": "$/cancelRequest", "params": { "id": i } });
        assert!(dispatch_client(cancel, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap());
    }

    let lock = can.lock().unwrap();
    let expected: HashSet<_> = (500..510).map(|i| json!(i)).collect();
    assert_eq!(*lock, expected);
}
