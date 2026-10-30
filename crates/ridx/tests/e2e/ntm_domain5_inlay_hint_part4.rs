/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for inlayHint/resolve field preservation (Domain 5: Intelligence & Completion).

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

    let init_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "capabilities": {}
        }
    });
    let _ = dispatch_lsp_message(LspMessage::Client(init_req), &mut out, &mut lc, &mut mr, &mut ov, &can, None);
    let init_notif = json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {}
    });
    let _ = dispatch_lsp_message(LspMessage::Client(init_notif), &mut out, &mut lc, &mut mr, &mut ov, &can, None);
    (mr, ov, lc, can)
}

fn send_inlay_hint_resolve_req(
    params: serde_json::Value,
    mr: &mut MultiRootWorkspace,
    ov: &mut OverlayManager,
    lc: &mut LifecycleManager,
    can: &Arc<Mutex<HashSet<serde_json::Value>>>,
) -> serde_json::Value {
    let req = json!({
        "jsonrpc": "2.0",
        "id": 62,
        "method": "inlayHint/resolve",
        "params": params
    });
    let mut out = Vec::new();
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None);
    extract_response(&out)
}

#[test]
fn test_inlay_hint_resolve_preserves_padding_right() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "position": { "line": 2, "character": 5 },
        "label": ": u128",
        "paddingRight": true
    });
    let res = send_inlay_hint_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["paddingRight"], true);
}

#[test]
fn test_inlay_hint_resolve_preserves_data_payload_string() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "position": { "line": 1, "character": 4 },
        "label": ": &str",
        "data": "opaque_identifier_string"
    });
    let res = send_inlay_hint_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["data"], "opaque_identifier_string");
}

#[test]
fn test_inlay_hint_resolve_preserves_data_payload_object() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "position": { "line": 10, "character": 1 },
        "label": ": Result<(), Error>",
        "data": { "file_id": 99, "symbol_idx": 4 }
    });
    let res = send_inlay_hint_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["data"]["file_id"], 99);
    assert_eq!(res["result"]["data"]["symbol_idx"], 4);
}

#[test]
fn test_inlay_hint_resolve_preserves_data_payload_array() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "position": { "line": 7, "character": 9 },
        "label": ": [u8; 32]",
        "data": [1, 2, 3]
    });
    let res = send_inlay_hint_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["data"], json!([1, 2, 3]));
}

#[test]
fn test_inlay_hint_resolve_cancelled_request_returns_cancelled_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let cancel_id = json!(66);
    can.lock().unwrap().insert(cancel_id.clone());

    let req = json!({
        "jsonrpc": "2.0",
        "id": cancel_id,
        "method": "inlayHint/resolve",
        "params": {
            "position": { "line": 0, "character": 0 },
            "label": ": u32"
        }
    });
    let mut out = Vec::new();
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, &mut lc, &mut mr, &mut ov, &can, None);
    let res = extract_response(&out);
    assert_eq!(res["error"]["code"], -32800);
}

#[test]
fn test_inlay_hint_resolve_idempotent_multiple_calls() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "position": { "line": 4, "character": 10 },
        "label": ": f64"
    });
    let res1 = send_inlay_hint_resolve_req(params.clone(), &mut mr, &mut ov, &mut lc, &can);
    let res2 = send_inlay_hint_resolve_req(res1["result"].clone(), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res1["result"], res2["result"]);
    assert_eq!(res2["result"]["tooltip"], "Inferred type by ridx");
}

#[test]
fn test_inlay_hint_resolve_utf16_position_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "position": { "line": 15, "character": 128 },
        "label": ": char"
    });
    let res = send_inlay_hint_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["position"]["character"], 128);
}

#[test]
fn test_inlay_hint_resolve_multiline_label_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "position": { "line": 0, "character": 1 },
        "label": ": (u32,\n u64)"
    });
    let res = send_inlay_hint_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["label"], ": (u32,\n u64)");
}

#[test]
fn test_inlay_hint_resolve_empty_string_tooltip_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "position": { "line": 1, "character": 2 },
        "label": ": ()",
        "tooltip": ""
    });
    let res = send_inlay_hint_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["tooltip"], "");
}

#[test]
fn test_inlay_hint_resolve_with_text_edits_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "position": { "line": 0, "character": 5 },
        "label": ": usize",
        "textEdits": [{
            "range": {
                "start": { "line": 0, "character": 5 },
                "end": { "line": 0, "character": 5 }
            },
            "newText": ": usize"
        }]
    });
    let res = send_inlay_hint_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["textEdits"][0]["newText"], ": usize");
}
