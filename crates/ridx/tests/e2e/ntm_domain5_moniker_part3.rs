/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/moniker definitions and cancellation (Domain 5: Intelligence & Completion).

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

fn send_moniker_req(
    uri: &str,
    line: u32,
    char_col: u32,
    mr: &mut MultiRootWorkspace,
    ov: &mut OverlayManager,
    lc: &mut LifecycleManager,
    can: &Arc<Mutex<HashSet<serde_json::Value>>>,
) -> serde_json::Value {
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 80,
        "method": "textDocument/moniker",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": char_col }
        }
    });
    dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None).unwrap();
    extract_response(&out)
}

fn open_test_file(ov: &mut OverlayManager, uri: &str, version: i32, text: &str) {
    let file = ridx::lsp::protocol::uri_to_path(uri);
    ov.open_or_update(uri, &file, version, text.to_string());
}

#[test]
fn test_moniker_missing_params_defaults_empty() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 81,
        "method": "textDocument/moniker"
    });
    dispatch_lsp_message(LspMessage::Client(req), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
    let res = extract_response(&out);
    assert_eq!(res["result"], json!([]));
}

#[test]
fn test_moniker_missing_text_document_defaults_empty() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 82,
        "method": "textDocument/moniker",
        "params": {
            "position": { "line": 0, "character": 0 }
        }
    });
    dispatch_lsp_message(LspMessage::Client(req), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
    let res = extract_response(&out);
    assert_eq!(res["result"], json!([]));
}

#[test]
fn test_moniker_missing_position_defaults_empty() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/m_no_pos.rs";
    open_test_file(&mut ov, uri, 1, "fn sample() {}\n");

    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 83,
        "method": "textDocument/moniker",
        "params": {
            "textDocument": { "uri": uri }
        }
    });
    dispatch_lsp_message(LspMessage::Client(req), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
    let res = extract_response(&out);
    assert_eq!(res["result"], json!([]));
}

#[test]
fn test_moniker_cancelled_request_returns_cancelled_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let cancel_id = json!(84);
    can.lock().unwrap().insert(cancel_id.clone());

    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": cancel_id,
        "method": "textDocument/moniker",
        "params": {
            "textDocument": { "uri": "file:///tmp/m_cancel.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });
    dispatch_lsp_message(LspMessage::Client(req), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
    let res = extract_response(&out);
    assert_eq!(res["error"]["code"], -32800);
}

#[test]
fn test_moniker_resolve_cancelled_request_returns_cancelled_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let cancel_id = json!(85);
    can.lock().unwrap().insert(cancel_id.clone());

    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": cancel_id,
        "method": "moniker/resolve",
        "params": {
            "scheme": "rust",
            "identifier": "core::fmt",
            "unique": "project"
        }
    });
    dispatch_lsp_message(LspMessage::Client(req), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
    let res = extract_response(&out);
    assert_eq!(res["error"]["code"], -32800);
}

#[test]
fn test_moniker_union_definition_moniker() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/m_union.rs";
    open_test_file(&mut ov, uri, 1, "union RawPayload {\n    f1: u32,\n    f2: f32,\n}\n");

    let res = send_moniker_req(uri, 0, 7, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["scheme"], "rust");
    assert_eq!(items[0]["unique"], "project");
    assert_eq!(items[0]["identifier"], "/tmp/m_union.rs::RawPayload");
}

#[test]
fn test_moniker_static_item_moniker() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/m_static.rs";
    open_test_file(&mut ov, uri, 1, "static APP_STATE: i32 = 0;\n");

    let res = send_moniker_req(uri, 0, 8, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["scheme"], "rust");
    assert_eq!(items[0]["unique"], "project");
    assert_eq!(items[0]["identifier"], "/tmp/m_static.rs::APP_STATE");
}

#[test]
fn test_moniker_macro_rules_position_empty() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/m_macro.rs";
    open_test_file(&mut ov, uri, 1, "macro_rules! my_macro { () => {} }\n");

    let res = send_moniker_req(uri, 0, 15, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 0);
}

#[test]
fn test_moniker_submodule_definition_moniker() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/m_submod.rs";
    open_test_file(&mut ov, uri, 1, "mod internal_engine {\n    pub fn run() {}\n}\n");

    let res = send_moniker_req(uri, 0, 5, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["scheme"], "rust");
    assert_eq!(items[0]["unique"], "project");
    assert_eq!(items[0]["identifier"], "/tmp/m_submod.rs::internal_engine");
}

#[test]
fn test_moniker_closed_document_cleared_from_overlay() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/m_close.rs";
    open_test_file(&mut ov, uri, 1, "fn ephemeral() {}\n");

    let file = ridx::lsp::protocol::uri_to_path(uri);
    ov.close(&file);

    let res = send_moniker_req(uri, 0, 3, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"], json!([]));
}
