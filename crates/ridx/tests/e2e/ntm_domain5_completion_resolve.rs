/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for completionItem/resolve request (Domain 5: Intelligence & Completion).

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
        "id": 10,
        "method": "completionItem/resolve",
        "params": params
    });
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None);
    extract_response(&out)
}

#[test]
fn test_completion_resolve_label_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(json!({ "label": "my_function" }), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["id"], 10);
    assert_eq!(val["result"]["label"], "my_function");
}

#[test]
fn test_completion_resolve_detail_to_markdown_documentation() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(
        json!({ "label": "calculate", "detail": "fn calculate(x: i32) -> i32" }),
        &mut mr, &mut ov, &mut lc, &can,
    );
    assert_eq!(val["result"]["label"], "calculate");
    assert_eq!(val["result"]["detail"], "fn calculate(x: i32) -> i32");
    assert_eq!(val["result"]["documentation"]["kind"], "markdown");
    assert_eq!(
        val["result"]["documentation"]["value"],
        "```rust\nfn calculate(x: i32) -> i32\n```"
    );
}

#[test]
fn test_completion_resolve_data_to_markdown_documentation() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(
        json!({ "label": "MyStruct", "data": "Full documentation text for MyStruct" }),
        &mut mr, &mut ov, &mut lc, &can,
    );
    assert_eq!(val["result"]["label"], "MyStruct");
    assert_eq!(val["result"]["documentation"]["kind"], "markdown");
    assert_eq!(
        val["result"]["documentation"]["value"],
        "Full documentation text for MyStruct"
    );
}

#[test]
fn test_completion_resolve_existing_doc_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(
        json!({
            "label": "foo",
            "detail": "fn foo()",
            "documentation": { "kind": "markdown", "value": "Original doc" }
        }),
        &mut mr, &mut ov, &mut lc, &can,
    );
    assert_eq!(val["result"]["documentation"]["value"], "Original doc");
}

#[test]
fn test_completion_resolve_empty_label_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(json!({ "label": "" }), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["error"]["code"], -32602);
}

#[test]
fn test_completion_resolve_missing_label_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(json!({ "detail": "fn foo()" }), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["error"]["code"], -32602);
}

#[test]
fn test_completion_resolve_missing_params_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(serde_json::Value::Null, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["error"]["code"], -32602);
}

#[test]
fn test_completion_resolve_non_object_params_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(json!("just a string"), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["error"]["code"], -32602);
}

#[test]
fn test_completion_resolve_kind_field_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(json!({ "label": "bar", "kind": 3 }), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["result"]["kind"], 3);
}

#[test]
fn test_completion_resolve_insert_text_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(
        json!({ "label": "func", "insertText": "func($1)" }),
        &mut mr, &mut ov, &mut lc, &can,
    );
    assert_eq!(val["result"]["insertText"], "func($1)");
}
