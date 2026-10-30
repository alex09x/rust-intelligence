/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/signatureHelp request part 2 (Domain 5: Intelligence & Completion).

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
        "id": 41,
        "method": "textDocument/signatureHelp",
        "params": params
    });
    let mut out = Vec::new();
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None);
    extract_response(&out)
}

#[test]
fn test_signature_help_outside_call_returns_null() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_outside.rs";
    open_test_file(&mut ov, uri, 1, "pub fn helper() {}\nlet x = 10;\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 4 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert!(res["result"].is_null());
}

#[test]
fn test_signature_help_after_closed_paren_returns_null() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_closed.rs";
    open_test_file(&mut ov, uri, 1, "pub fn helper(v: i32) {}\nfn caller() { helper(42); }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 26 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert!(res["result"].is_null());
}

#[test]
fn test_signature_help_turbofish_function_call() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_turbo.rs";
    open_test_file(&mut ov, uri, 1, "pub fn collect_data<T>(seed: T) -> Vec<T> { Vec::new() }\nfn run() { collect_data::<i32>( }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 32 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["signatures"][0]["label"], "fn collect_data<T>(seed: T) -> Vec<T>");
    assert_eq!(res["result"]["activeParameter"], 0);
}

#[test]
fn test_signature_help_whitespace_between_callee_and_paren() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_ws.rs";
    open_test_file(&mut ov, uri, 1, "pub fn spaced(x: u32) {}\nfn caller() { spaced   ( }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 25 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["signatures"][0]["label"], "fn spaced(x: u32)");
    assert_eq!(res["result"]["activeParameter"], 0);
}

#[test]
fn test_signature_help_multiline_call_arguments() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_multiline.rs";
    open_test_file(&mut ov, uri, 1, "pub fn configure(port: u16, host: &str) {}\nfn run() {\n    configure(\n        8080,\n        \n    );\n}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 4, "character": 8 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["activeParameter"], 1);
    assert_eq!(res["result"]["signatures"][0]["parameters"][1]["label"], "host: &str");
}

#[test]
fn test_signature_help_parameters_extracted_labels() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_params.rs";
    open_test_file(&mut ov, uri, 1, "pub fn render(width: u32, height: u32) {}\nfn test() { render( }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 20 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    let sig = &res["result"]["signatures"][0];
    let params_arr = sig["parameters"].as_array().expect("array");
    assert_eq!(params_arr.len(), 2);
    assert_eq!(params_arr[0]["label"], "width: u32");
    assert_eq!(params_arr[1]["label"], "height: u32");
}

#[test]
fn test_signature_help_active_signature_is_zero() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_zero.rs";
    open_test_file(&mut ov, uri, 1, "pub fn compute(val: f64) {}\nfn run() { compute( }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 20 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"]["activeSignature"], 0);
}

#[test]
fn test_signature_help_cancelled_request_returns_cancelled_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    {
        let mut guard = can.lock().unwrap();
        guard.insert(json!(41));
    }
    let params = json!({
        "textDocument": { "uri": "file:///tmp/sig_cancelled.rs" },
        "position": { "line": 0, "character": 0 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], -32800);
}

#[test]
fn test_signature_help_empty_document_returns_null() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_empty_doc.rs";
    open_test_file(&mut ov, uri, 1, "");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 0 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert!(res["result"].is_null());
}

#[test]
fn test_signature_help_nonexistent_file_returns_null() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "textDocument": { "uri": "file:///tmp/ghost_sig.rs" },
        "position": { "line": 0, "character": 0 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert!(res["result"].is_null());
}
