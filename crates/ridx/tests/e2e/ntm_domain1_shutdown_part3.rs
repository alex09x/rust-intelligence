/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for shutdown request part 3 (Domain 1: Lifecycle & Control).

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
        "id": 999,
        "method": method,
        "params": params
    });
    let _ = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    assert_eq!(val["id"], 999);
    assert_eq!(val["error"]["code"], -32600);
}

#[test]
fn test_shutdown_rejects_document_highlight_code_32600() {
    assert_rejected_with_32600(
        "textDocument/documentHighlight",
        json!({
            "textDocument": { "uri": "file:///workspace/lib.rs" },
            "position": { "line": 0, "character": 0 }
        }),
    );
}

#[test]
fn test_shutdown_rejects_implementation_post_shutdown_32600() {
    assert_rejected_with_32600(
        "textDocument/implementation",
        json!({
            "textDocument": { "uri": "file:///workspace/lib.rs" },
            "position": { "line": 0, "character": 0 }
        }),
    );
}

#[test]
fn test_shutdown_rejects_call_hierarchy_prepare_code_32600() {
    assert_rejected_with_32600(
        "textDocument/prepareCallHierarchy",
        json!({
            "textDocument": { "uri": "file:///workspace/lib.rs" },
            "position": { "line": 0, "character": 0 }
        }),
    );
}

#[test]
fn test_shutdown_rejects_type_hierarchy_prepare_code_32600() {
    assert_rejected_with_32600(
        "textDocument/prepareTypeHierarchy",
        json!({
            "textDocument": { "uri": "file:///workspace/lib.rs" },
            "position": { "line": 0, "character": 0 }
        }),
    );
}

#[test]
fn test_shutdown_rejects_folding_range_code_32600() {
    assert_rejected_with_32600(
        "textDocument/foldingRange",
        json!({
            "textDocument": { "uri": "file:///workspace/lib.rs" }
        }),
    );
}

#[test]
fn test_shutdown_rejects_selection_range_code_32600() {
    assert_rejected_with_32600(
        "textDocument/selectionRange",
        json!({
            "textDocument": { "uri": "file:///workspace/lib.rs" },
            "positions": [{ "line": 0, "character": 0 }]
        }),
    );
}

#[test]
fn test_shutdown_rejects_inlay_hint_post_shutdown_32600() {
    assert_rejected_with_32600(
        "textDocument/inlayHint",
        json!({
            "textDocument": { "uri": "file:///workspace/lib.rs" },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 10, "character": 0 }
            }
        }),
    );
}

#[test]
fn test_shutdown_rejects_semantic_tokens_code_32600() {
    assert_rejected_with_32600(
        "textDocument/semanticTokens/full",
        json!({
            "textDocument": { "uri": "file:///workspace/lib.rs" }
        }),
    );
}

#[test]
fn test_shutdown_rejects_prepare_rename_code_32600() {
    assert_rejected_with_32600(
        "textDocument/prepareRename",
        json!({
            "textDocument": { "uri": "file:///workspace/lib.rs" },
            "position": { "line": 0, "character": 0 }
        }),
    );
}

#[test]
fn test_shutdown_rejects_document_diagnostic_code_32600() {
    assert_rejected_with_32600(
        "textDocument/diagnostic",
        json!({
            "textDocument": { "uri": "file:///workspace/lib.rs" }
        }),
    );
}
