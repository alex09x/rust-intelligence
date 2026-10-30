/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/signatureHelp request (Domain 5: Intelligence & Completion).

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

fn open_test_file(ov: &mut OverlayManager, uri: &str, version: i32, text: &str) {
    let file = ridx::lsp::protocol::uri_to_path(uri);
    ov.open_or_update(uri, &file, version, text.to_string());
}

fn send_sig_help_req(
    params: serde_json::Value,
    mr: &mut MultiRootWorkspace,
    ov: &mut OverlayManager,
    lc: &mut LifecycleManager,
    can: &Arc<Mutex<HashSet<serde_json::Value>>>,
) -> serde_json::Value {
    let req = json!({
        "jsonrpc": "2.0",
        "id": 40,
        "method": "textDocument/signatureHelp",
        "params": params
    });
    let mut out = Vec::new();
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None);
    extract_response(&out)
}

#[test]
fn test_signature_help_single_param_fn_active_zero() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_single.rs";
    open_test_file(&mut ov, uri, 1, "pub fn process_value(x: i32) -> i32 { x }\nfn caller() { process_value( }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 29 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["activeSignature"], 0);
    assert_eq!(res["result"]["activeParameter"], 0);
    assert_eq!(res["result"]["signatures"][0]["label"], "fn process_value(x: i32) -> i32");
    assert_eq!(res["result"]["signatures"][0]["parameters"][0]["label"], "x: i32");
}

#[test]
fn test_signature_help_multi_param_fn_first_param() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_first.rs";
    open_test_file(&mut ov, uri, 1, "pub fn add(a: i32, b: i32) -> i32 { a + b }\nfn call() { add( }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 16 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["activeParameter"], 0);
    assert_eq!(res["result"]["signatures"][0]["parameters"][0]["label"], "a: i32");
}

#[test]
fn test_signature_help_multi_param_fn_second_param() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_second.rs";
    open_test_file(&mut ov, uri, 1, "pub fn add(a: i32, b: i32) -> i32 { a + b }\nfn call() { add(10,  }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 20 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["activeParameter"], 1);
    assert_eq!(res["result"]["signatures"][0]["parameters"][1]["label"], "b: i32");
}

#[test]
fn test_signature_help_multi_param_fn_third_param() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_third.rs";
    open_test_file(&mut ov, uri, 1, "pub fn clamp(val: i32, min: i32, max: i32) -> i32 { val }\nfn call() { clamp(10, 0,  }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 25 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["activeParameter"], 2);
    assert_eq!(res["result"]["signatures"][0]["parameters"][2]["label"], "max: i32");
}

#[test]
fn test_signature_help_no_param_fn() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_empty.rs";
    open_test_file(&mut ov, uri, 1, "pub fn init_engine() {}\nfn call() { init_engine( }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 24 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["activeParameter"], 0);
    let params_arr = res["result"]["signatures"][0]["parameters"].as_array().expect("array");
    assert_eq!(params_arr.len(), 0);
}

#[test]
fn test_signature_help_nested_call_inner_active_param() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_nest.rs";
    open_test_file(&mut ov, uri, 1, "pub fn outer(a: i32, b: i32) {}\npub fn inner(x: i32) -> i32 { x }\nfn call() { outer(inner( }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 2, "character": 24 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["signatures"][0]["label"], "fn inner(x: i32) -> i32");
    assert_eq!(res["result"]["activeParameter"], 0);
}

#[test]
fn test_signature_help_nested_call_outer_second_param() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_nest2.rs";
    open_test_file(&mut ov, uri, 1, "pub fn outer(a: i32, b: i32) {}\npub fn inner(x: i32) -> i32 { x }\nfn call() { outer(inner(42),  }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 2, "character": 29 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["signatures"][0]["label"], "fn outer(a: i32, b: i32)");
    assert_eq!(res["result"]["activeParameter"], 1);
}

#[test]
fn test_signature_help_missing_params_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let res = send_sig_help_req(serde_json::Value::Null, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], -32602);
}

#[test]
fn test_signature_help_missing_text_document_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "position": { "line": 0, "character": 0 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], -32602);
}

#[test]
fn test_signature_help_missing_position_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "textDocument": { "uri": "file:///tmp/sig_test.rs" }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], -32602);
}
