/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for shutdown request part 2 (Domain 1: Lifecycle & Control).

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

fn setup_shutdown_server() -> (MultiRootWorkspace, OverlayManager, LifecycleManager, Arc<Mutex<HashSet<serde_json::Value>>>) {
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
    out.clear();
    let shut = json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" });
    dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
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
fn test_shutdown_rejects_workspace_symbol_code_32600() {
    let (mut mr, mut ov, mut lc, can) = setup_shutdown_server();
    let mut out = Vec::new();

    let req = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "workspace/symbol",
        "params": { "query": "test" }
    });
    let _ = dispatch_client(req, &mut out, &mut mr, &mut ov, &mut lc, &can);

    let val = extract_response(&out);
    assert_eq!(val["id"], 3);
    assert_eq!(val["error"]["code"], -32600);
}

#[test]
fn test_shutdown_rejects_rename_code_32600() {
    let (mut mr, mut ov, mut lc, can) = setup_shutdown_server();
    let mut out = Vec::new();

    let req = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": "file:///tmp/dummy.rs" },
            "position": { "line": 0, "character": 0 },
            "newName": "renamed"
        }
    });
    let _ = dispatch_client(req, &mut out, &mut mr, &mut ov, &mut lc, &can);

    let val = extract_response(&out);
    assert_eq!(val["id"], 4);
    assert_eq!(val["error"]["code"], -32600);
}

#[test]
fn test_shutdown_rejects_formatting_code_32600() {
    let (mut mr, mut ov, mut lc, can) = setup_shutdown_server();
    let mut out = Vec::new();

    let req = json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "textDocument/formatting",
        "params": {
            "textDocument": { "uri": "file:///tmp/dummy.rs" },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    let _ = dispatch_client(req, &mut out, &mut mr, &mut ov, &mut lc, &can);

    let val = extract_response(&out);
    assert_eq!(val["id"], 5);
    assert_eq!(val["error"]["code"], -32600);
}

#[test]
fn test_shutdown_rejects_code_action_code_32600() {
    let (mut mr, mut ov, mut lc, can) = setup_shutdown_server();
    let mut out = Vec::new();

    let req = json!({
        "jsonrpc": "2.0",
        "id": 6,
        "method": "textDocument/codeAction",
        "params": {
            "textDocument": { "uri": "file:///tmp/dummy.rs" },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } },
            "context": { "diagnostics": [] }
        }
    });
    let _ = dispatch_client(req, &mut out, &mut mr, &mut ov, &mut lc, &can);

    let val = extract_response(&out);
    assert_eq!(val["id"], 6);
    assert_eq!(val["error"]["code"], -32600);
}

#[test]
fn test_shutdown_rejects_call_hierarchy_code_32600() {
    let (mut mr, mut ov, mut lc, can) = setup_shutdown_server();
    let mut out = Vec::new();

    let req = json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": "file:///tmp/dummy.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });
    let _ = dispatch_client(req, &mut out, &mut mr, &mut ov, &mut lc, &can);

    let val = extract_response(&out);
    assert_eq!(val["id"], 7);
    assert_eq!(val["error"]["code"], -32600);
}

#[test]
fn test_shutdown_rejects_type_hierarchy_code_32600() {
    let (mut mr, mut ov, mut lc, can) = setup_shutdown_server();
    let mut out = Vec::new();

    let req = json!({
        "jsonrpc": "2.0",
        "id": 8,
        "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": "file:///tmp/dummy.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });
    let _ = dispatch_client(req, &mut out, &mut mr, &mut ov, &mut lc, &can);

    let val = extract_response(&out);
    assert_eq!(val["id"], 8);
    assert_eq!(val["error"]["code"], -32600);
}

#[test]
fn test_shutdown_rejects_inlay_hint_code_32600() {
    let (mut mr, mut ov, mut lc, can) = setup_shutdown_server();
    let mut out = Vec::new();

    let req = json!({
        "jsonrpc": "2.0",
        "id": 9,
        "method": "textDocument/inlayHint",
        "params": {
            "textDocument": { "uri": "file:///tmp/dummy.rs" },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } }
        }
    });
    let _ = dispatch_client(req, &mut out, &mut mr, &mut ov, &mut lc, &can);

    let val = extract_response(&out);
    assert_eq!(val["id"], 9);
    assert_eq!(val["error"]["code"], -32600);
}

#[test]
fn test_shutdown_flycheck_state_quiesced() {
    let (_, _, lc, _) = setup_shutdown_server();
    assert!(lc.is_shutdown());
}

#[test]
fn test_shutdown_preserves_request_id() {
    let (mut mr, mut ov, mut lc, can) = setup_shutdown_server();
    let mut out = Vec::new();

    let req = json!({
        "jsonrpc": "2.0",
        "id": "custom-string-id-999",
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": "file:///tmp/dummy.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });
    let _ = dispatch_client(req, &mut out, &mut mr, &mut ov, &mut lc, &can);

    let val = extract_response(&out);
    assert_eq!(val["id"], "custom-string-id-999");
    assert_eq!(val["error"]["code"], -32600);
}

#[test]
fn test_shutdown_allows_exit_after() {
    let (mut mr, mut ov, mut lc, can) = setup_shutdown_server();
    let mut out = Vec::new();

    let exit_msg = json!({ "jsonrpc": "2.0", "method": "exit" });
    let res = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(!res.unwrap());
    assert_eq!(out, b"");
}
