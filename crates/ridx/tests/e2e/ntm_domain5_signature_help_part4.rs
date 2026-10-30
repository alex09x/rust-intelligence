/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/signatureHelp request part 4 (Domain 5: Intelligence & Completion).

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
        "id": 43,
        "method": "textDocument/signatureHelp",
        "params": params
    });
    let mut out = Vec::new();
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None);
    extract_response(&out)
}

#[test]
fn test_signature_help_dirty_overlay_updates_signature() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_dirty.rs";
    open_test_file(&mut ov, uri, 1, "pub fn calculate(val: i32) {}\nfn caller() { calculate( }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 25 }
    });
    let res1 = send_sig_help_req(params.clone(), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res1["result"]["signatures"][0]["parameters"][0]["label"], "val: i32");

    // Update overlay with new parameter type
    open_test_file(&mut ov, uri, 2, "pub fn calculate(val: f64) {}\nfn caller() { calculate( }\n");
    let res2 = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res2["result"]["signatures"][0]["parameters"][0]["label"], "val: f64");
}

#[test]
fn test_signature_help_closed_document_cleared_from_overlay() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_closed_doc.rs";
    let file = ridx::lsp::protocol::uri_to_path(uri);
    ov.open_or_update(uri, &file, 1, "pub fn target_api(code: u32) {}\nfn run() { target_api( }\n".to_string());

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 23 }
    });
    let res1 = send_sig_help_req(params.clone(), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res1["result"]["activeParameter"], 0);

    ov.close(&file);
    let res2 = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res2["result"].is_null());
}

#[test]
fn test_signature_help_method_call_on_self() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_self.rs";
    open_test_file(&mut ov, uri, 1, "struct Task;\nimpl Task {\n    pub fn execute(&self, priority: u8) {}\n    fn run(&self) { self.execute( }\n}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 3, "character": 34 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["activeParameter"], 0);
    assert_eq!(res["result"]["signatures"][0]["label"], "fn execute(&self, priority: u8)");
}

#[test]
fn test_signature_help_multiple_sequential_requests_stable() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_stable.rs";
    open_test_file(&mut ov, uri, 1, "pub fn stable_op(code: usize) {}\nfn caller() { stable_op( }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 24 }
    });
    for _ in 0..5 {
        let res = send_sig_help_req(params.clone(), &mut mr, &mut ov, &mut lc, &can);
        assert_eq!(res["result"]["activeParameter"], 0);
        assert_eq!(res["result"]["signatures"][0]["label"], "fn stable_op(code: usize)");
    }
}

#[test]
fn test_signature_help_trailing_comma_active_parameter() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_trailing.rs";
    open_test_file(&mut ov, uri, 1, "pub fn build_three(a: i32, b: i32, c: i32) {}\nfn run() { build_three(1, 2,  }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 30 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["activeParameter"], 2);
}

#[test]
fn test_signature_help_string_literal_in_argument() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_str_arg.rs";
    open_test_file(&mut ov, uri, 1, "pub fn format_text(header: &str, footer: &str) {}\nfn run() { format_text(\"alpha\", }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 31 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["activeParameter"], 1);
    assert_eq!(res["result"]["signatures"][0]["parameters"][1]["label"], "footer: &str");
}

#[test]
fn test_signature_help_return_type_arrow_present_in_label() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_arrow.rs";
    open_test_file(&mut ov, uri, 1, "pub fn query_count() -> usize { 0 }\nfn run() { query_count( }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 24 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"]["signatures"][0]["label"], "fn query_count() -> usize");
}

#[test]
fn test_signature_help_generic_function_signature_label() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_generic_sig.rs";
    open_test_file(&mut ov, uri, 1, "pub fn box_item<T>(val: T) -> Box<T> { Box::new(val) }\nfn run() { box_item( }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 20 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"]["signatures"][0]["label"], "fn box_item<T>(val: T) -> Box<T>");
}

#[test]
fn test_signature_help_comment_inside_arg_list() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_comment.rs";
    open_test_file(&mut ov, uri, 1, "pub fn calculate(factor: f32, offset: f32) {}\nfn run() { calculate(/* config */ 1.0,  }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 40 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["activeParameter"], 1);
}

#[test]
fn test_signature_help_tab_indented_call() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_tab.rs";
    open_test_file(&mut ov, uri, 1, "pub fn tabbed_fn(x: i32) {}\nfn caller() {\n\ttabbed_fn(\n}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 2, "character": 11 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["signatures"][0]["label"], "fn tabbed_fn(x: i32)");
}
