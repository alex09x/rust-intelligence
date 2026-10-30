/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for inlayHint/resolve request (Domain 5: Intelligence & Completion).

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
        "id": 60,
        "method": "inlayHint/resolve",
        "params": params
    });
    let mut out = Vec::new();
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None);
    extract_response(&out)
}

#[test]
fn test_inlay_hint_resolve_missing_params_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 61,
        "method": "inlayHint/resolve"
    });
    let mut out = Vec::new();
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, &mut lc, &mut mr, &mut ov, &can, None);
    let res = extract_response(&out);
    assert_eq!(res["error"]["code"], -32602);
}

#[test]
fn test_inlay_hint_resolve_non_object_params_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!("not-an-object");
    let res = send_inlay_hint_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], -32602);
}

#[test]
fn test_inlay_hint_resolve_missing_position_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "label": ": u32"
    });
    let res = send_inlay_hint_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], -32602);
}

#[test]
fn test_inlay_hint_resolve_missing_label_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "position": { "line": 0, "character": 5 }
    });
    let res = send_inlay_hint_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], -32602);
}

#[test]
fn test_inlay_hint_resolve_populates_default_tooltip() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "position": { "line": 1, "character": 8 },
        "label": ": String"
    });
    let res = send_inlay_hint_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["tooltip"], "Inferred type by ridx");
}

#[test]
fn test_inlay_hint_resolve_preserves_custom_tooltip() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "position": { "line": 2, "character": 4 },
        "label": ": usize",
        "tooltip": "Custom provided type description"
    });
    let res = send_inlay_hint_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["tooltip"], "Custom provided type description");
}

#[test]
fn test_inlay_hint_resolve_preserves_position() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "position": { "line": 42, "character": 17 },
        "label": ": i64"
    });
    let res = send_inlay_hint_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["position"]["line"], 42);
    assert_eq!(res["result"]["position"]["character"], 17);
}

#[test]
fn test_inlay_hint_resolve_preserves_label() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "position": { "line": 0, "character": 3 },
        "label": ": Vec<u8>"
    });
    let res = send_inlay_hint_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["label"], ": Vec<u8>");
}

#[test]
fn test_inlay_hint_resolve_preserves_kind() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "position": { "line": 3, "character": 12 },
        "label": ": bool",
        "kind": 1
    });
    let res = send_inlay_hint_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["kind"], 1);
}

#[test]
fn test_inlay_hint_resolve_preserves_padding_left() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "position": { "line": 5, "character": 2 },
        "label": ": Option<i32>",
        "paddingLeft": true
    });
    let res = send_inlay_hint_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["paddingLeft"], true);
}
