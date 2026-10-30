/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for completionItem/resolve request part 4 (Domain 5: Intelligence & Completion).

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

fn send_resolve_req(
    params: serde_json::Value,
    mr: &mut MultiRootWorkspace,
    ov: &mut OverlayManager,
    lc: &mut LifecycleManager,
    can: &Arc<Mutex<HashSet<serde_json::Value>>>,
) -> serde_json::Value {
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 20,
        "method": "completionItem/resolve",
        "params": params
    });
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None);
    extract_response(&out)
}

#[test]
fn test_completion_resolve_integer_label_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(json!({ "label": 12345 }), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["error"]["code"], -32602);
}

#[test]
fn test_completion_resolve_boolean_label_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(json!({ "label": true }), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["error"]["code"], -32602);
}

#[test]
fn test_completion_resolve_null_label_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(json!({ "label": null }), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["error"]["code"], -32602);
}

#[test]
fn test_completion_resolve_array_params_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(json!([{ "label": "item_in_array" }]), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["error"]["code"], -32602);
}

#[test]
fn test_completion_resolve_string_params_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(json!("raw_string_param"), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["error"]["code"], -32602);
}

#[test]
fn test_completion_resolve_numeric_params_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(json!(42), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["error"]["code"], -32602);
}

#[test]
fn test_completion_resolve_null_params_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(serde_json::Value::Null, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["error"]["code"], -32602);
}

#[test]
fn test_completion_resolve_boolean_params_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(json!(false), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["error"]["code"], -32602);
}

#[test]
fn test_completion_resolve_extra_unknown_fields_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(
        json!({
            "label": "custom_item",
            "x_custom_metadata": {
                "origin": "compiler_pass",
                "score": 99
            }
        }),
        &mut mr,
        &mut ov,
        &mut lc,
        &can,
    );
    assert_eq!(val["result"]["x_custom_metadata"]["origin"], "compiler_pass");
    assert_eq!(val["result"]["x_custom_metadata"]["score"], 99);
}

#[test]
fn test_completion_resolve_multiline_detail_formatted_cleanly() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(
        json!({
            "label": "ConfigStruct",
            "detail": "pub struct Config {\n    pub port: u16,\n    pub host: String,\n}"
        }),
        &mut mr,
        &mut ov,
        &mut lc,
        &can,
    );
    assert_eq!(val["result"]["documentation"]["kind"], "markdown");
    assert_eq!(val["result"]["documentation"]["value"], "```rust\npub struct Config {\n    pub port: u16,\n    pub host: String,\n}\n```");
}
