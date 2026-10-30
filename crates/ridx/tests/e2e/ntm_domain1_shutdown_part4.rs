/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for shutdown request part 4 (Domain 1: Lifecycle & Control).

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

fn assert_rejected_with_32600(method: &str, params: serde_json::Value) {
    let (mut mr, mut ov, mut lc, can) = setup_shutdown_server();
    let mut out = Vec::new();
    let msg = json!({
        "jsonrpc": "2.0",
        "id": 888,
        "method": method,
        "params": params
    });
    let _ = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    assert_eq!(val["id"], 888);
    assert_eq!(val["error"]["code"], -32600);
}

#[test]
fn test_shutdown_rejects_workspace_diagnostic_code_32600() {
    assert_rejected_with_32600("workspace/diagnostic", json!({ "previousResultIds": [] }));
}

#[test]
fn test_shutdown_rejects_execute_command_code_32600() {
    assert_rejected_with_32600("workspace/executeCommand", json!({ "command": "rust-intelligence.apply", "arguments": [] }));
}

#[test]
fn test_shutdown_rejects_code_action_resolve_code_32600() {
    assert_rejected_with_32600("codeAction/resolve", json!({ "title": "Extract" }));
}

#[test]
fn test_shutdown_rejects_completion_resolve_code_32600() {
    assert_rejected_with_32600("completionItem/resolve", json!({ "label": "test" }));
}

#[test]
fn test_shutdown_rejects_workspace_symbol_resolve_code_32600() {
    assert_rejected_with_32600("workspaceSymbol/resolve", json!({ "name": "Item" }));
}

#[test]
fn test_shutdown_rejects_inlay_hint_resolve_code_32600() {
    assert_rejected_with_32600("inlayHint/resolve", json!({ "label": ": i32" }));
}

#[test]
fn test_shutdown_rejects_signature_help_code_32600() {
    assert_rejected_with_32600(
        "textDocument/signatureHelp",
        json!({
            "textDocument": { "uri": "file:///workspace/lib.rs" },
            "position": { "line": 0, "character": 0 }
        }),
    );
}

#[test]
fn test_shutdown_rejects_inline_value_code_32600() {
    assert_rejected_with_32600(
        "textDocument/inlineValue",
        json!({
            "textDocument": { "uri": "file:///workspace/lib.rs" },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 1, "character": 0 } }
        }),
    );
}

#[test]
fn test_shutdown_rejects_moniker_code_32600() {
    assert_rejected_with_32600(
        "textDocument/moniker",
        json!({
            "textDocument": { "uri": "file:///workspace/lib.rs" },
            "position": { "line": 0, "character": 0 }
        }),
    );
}

#[test]
fn test_shutdown_rejects_document_link_code_32600() {
    assert_rejected_with_32600(
        "textDocument/documentLink",
        json!({
            "textDocument": { "uri": "file:///workspace/lib.rs" }
        }),
    );
}
