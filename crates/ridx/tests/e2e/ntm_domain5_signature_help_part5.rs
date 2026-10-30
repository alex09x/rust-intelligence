/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/signatureHelp request part 5 (Domain 5: Intelligence & Completion).

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
        "id": 44,
        "method": "textDocument/signatureHelp",
        "params": params
    });
    let mut out = Vec::new();
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None);
    extract_response(&out)
}

#[test]
fn test_signature_help_ref_mut_parameter_labels() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_ref_mut.rs";
    open_test_file(&mut ov, uri, 1, "pub fn update_buffer(buf: &mut Vec<u8>) {}\nfn run() { update_buffer( }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 25 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["signatures"][0]["parameters"][0]["label"], "buf: &mut Vec<u8>");
}

#[test]
fn test_signature_help_lifetime_parameter_labels() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_lifetime.rs";
    open_test_file(&mut ov, uri, 1, "pub fn parse_slice<'a>(input: &'a str) -> &'a str { input }\nfn run() { parse_slice( }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 23 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["signatures"][0]["parameters"][0]["label"], "input: &'a str");
}

#[test]
fn test_signature_help_impl_trait_parameter_labels() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_impl_trait.rs";
    open_test_file(&mut ov, uri, 1, "pub fn consume_iter(items: impl Iterator<Item = u32>) {}\nfn run() { consume_iter( }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 24 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["signatures"][0]["parameters"][0]["label"], "items: impl Iterator<Item = u32>");
}

#[test]
fn test_signature_help_dyn_trait_parameter_labels() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_dyn_trait.rs";
    open_test_file(&mut ov, uri, 1, "pub fn run_task(task: &dyn Fn()) {}\nfn run() { run_task( }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 20 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["signatures"][0]["parameters"][0]["label"], "task: &dyn Fn()");
}

#[test]
fn test_signature_help_fn_pointer_parameter_labels() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_fn_ptr.rs";
    open_test_file(&mut ov, uri, 1, "pub fn map_int(f: fn(i32) -> i32, val: i32) -> i32 { f(val) }\nfn run() { map_int( }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 19 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["signatures"][0]["parameters"][0]["label"], "f: fn(i32) -> i32");
    assert_eq!(res["result"]["signatures"][0]["parameters"][1]["label"], "val: i32");
}

#[test]
fn test_signature_help_utf16_surrogate_in_call_line() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_emoji.rs";
    open_test_file(&mut ov, uri, 1, "// 🚀 launch\npub fn launch_target(count: u32) {}\nfn run() { launch_target( }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 2, "character": 25 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["activeParameter"], 0);
    assert_eq!(res["result"]["signatures"][0]["label"], "fn launch_target(count: u32)");
}

#[test]
fn test_signature_help_four_arguments_progression() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_four.rs";
    open_test_file(&mut ov, uri, 1, "pub fn quad(a: i32, b: i32, c: i32, d: i32) {}\n");

    for (args_str, expected_param) in [("quad(", 0), ("quad(1, ", 1), ("quad(1, 2, ", 2), ("quad(1, 2, 3, ", 3)] {
        let content = format!("pub fn quad(a: i32, b: i32, c: i32, d: i32) {{}}\nfn run() {{ {args_str} }}\n");
        open_test_file(&mut ov, uri, 2, &content);
        let col = 11 + args_str.len();
        let params = json!({
            "textDocument": { "uri": uri },
            "position": { "line": 1, "character": col }
        });
        let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
        assert_eq!(res["result"]["activeParameter"], expected_param);
    }
}

#[test]
fn test_signature_help_zero_signatures_when_null() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_null.rs";
    open_test_file(&mut ov, uri, 1, "pub struct Standalone;\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 5 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert!(res["result"].is_null());
}

#[test]
fn test_signature_help_preceding_text_large_buffer_handled() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/sig_large.rs";
    let mut large = String::new();
    for i in 0..100 {
        large.push_str(&format!("// filler comment line {i}\n"));
    }
    large.push_str("pub fn big_fn(size: usize) {}\nfn caller() { big_fn( }\n");
    open_test_file(&mut ov, uri, 1, &large);

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 101, "character": 21 }
    });
    let res = send_sig_help_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["signatures"][0]["label"], "fn big_fn(size: usize)");
}
