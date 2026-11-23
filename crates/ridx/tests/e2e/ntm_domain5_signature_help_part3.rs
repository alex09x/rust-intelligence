/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/signatureHelp request part 3 (Domain 5: Intelligence & Completion).

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
        "id": 42,
        "method": "textDocument/signatureHelp",
        "params": params
    });
    let mut out = Vec::new();
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None);
    extract_response(&out)
}

#[test]
fn test_signature_help_array_expression_in_argument() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_array.rs";
    open_test_file(&mut ov, uri, 1, "pub fn combine(list: &[i32], label: &str) {}\nfn run() { combine([1, 2, 3], }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 30 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["activeParameter"], 1);
    assert_eq!(res["result"]["signatures"][0]["parameters"][1]["label"], "label: &str");
}

#[test]
fn test_signature_help_closure_expression_in_argument() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_closure.rs";
    open_test_file(&mut ov, uri, 1, "pub fn apply_op(f: fn(i32) -> i32, count: usize) {}\nfn run() { apply_op(|x| x + 1, }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 30 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["activeParameter"], 1);
    assert_eq!(res["result"]["signatures"][0]["parameters"][1]["label"], "count: usize");
}

#[test]
fn test_signature_help_generic_type_args_in_argument() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_generic_args.rs";
    open_test_file(&mut ov, uri, 1, "pub fn construct(tuple: (i32, i32), tag: u32) {}\nfn run() { construct(tuple_gen::<i32, u32>(),  }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 46 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["activeParameter"], 1);
    assert_eq!(res["result"]["signatures"][0]["parameters"][1]["label"], "tag: u32");
}

#[test]
fn test_signature_help_tuple_expression_in_argument() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_tuple.rs";
    open_test_file(&mut ov, uri, 1, "pub fn pair_call(pair: (i32, i32), third: i32) {}\nfn run() { pair_call((10, 20),  }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 32 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["activeParameter"], 1);
    assert_eq!(res["result"]["signatures"][0]["parameters"][1]["label"], "third: i32");
}

#[test]
fn test_signature_help_block_expression_in_argument() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_block.rs";
    open_test_file(&mut ov, uri, 1, "pub fn compute_sum(a: i32, b: i32) {}\nfn run() { compute_sum({ let val = 1; val },  }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 46 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["activeParameter"], 1);
    assert_eq!(res["result"]["signatures"][0]["parameters"][1]["label"], "b: i32");
}

#[test]
fn test_signature_help_unknown_callee_returns_null() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_unknown.rs";
    open_test_file(&mut ov, uri, 1, "fn caller() { non_existent_fn(10,  }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 34 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert!(res["result"].is_null());
}

#[test]
fn test_signature_help_out_of_bounds_line_is_invalid_params() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_oob_l.rs";
    open_test_file(&mut ov, uri, 1, "pub fn helper() {}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 9999, "character": 0 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], -32602, "a line past the end is invalid params (issue #2): {res}");
}

#[test]
fn test_signature_help_out_of_bounds_character_returns_null() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_oob_c.rs";
    open_test_file(&mut ov, uri, 1, "pub fn helper() {}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 500 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert!(res["result"].is_null());
}

#[test]
fn test_signature_help_invalid_line_negative_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "textDocument": { "uri": "file:///tmp/sig_err.rs" },
        "position": { "line": -2, "character": 0 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], -32602);
}

#[test]
fn test_signature_help_invalid_character_negative_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "textDocument": { "uri": "file:///tmp/sig_err.rs" },
        "position": { "line": 0, "character": -4 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], -32602);
}
