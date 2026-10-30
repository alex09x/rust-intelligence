/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 2: `workspace/diagnostic` pull integration tests (part 4: multi-statement, line reporting, overlay clearing).

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::LspMessage;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

fn extract_response(out: &[u8]) -> Value {
    let s = String::from_utf8_lossy(out);
    for part in s.split("Content-Length: ") {
        if let Some(idx) = part.find("\r\n\r\n") {
            let body = &part[idx + 4..];
            if let Ok(v) = serde_json::from_str::<Value>(body.trim()) {
                return v;
            }
        }
    }
    Value::Null
}

fn setup_initialized_server() -> (MultiRootWorkspace, OverlayManager, LifecycleManager, Arc<Mutex<HashSet<Value>>>) {
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

fn send_ws_diagnostic_req(params: Value, mr: &mut MultiRootWorkspace, ov: &mut OverlayManager, lc: &mut LifecycleManager, can: &Arc<Mutex<HashSet<Value>>>) -> Value {
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 200,
        "method": "workspace/diagnostic",
        "params": params
    });
    dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None).unwrap();
    extract_response(&out)
}

fn open_test_file(ov: &mut OverlayManager, uri: &str, version: i32, text: &str) {
    let file = ridx::lsp::protocol::uri_to_path(uri);
    ov.open_or_update(uri, &file, version, text.to_string());
}

fn close_test_file(ov: &mut OverlayManager, uri: &str) {
    let file = ridx::lsp::protocol::uri_to_path(uri);
    ov.close(&file);
}

#[test]
fn test_workspace_diagnostic_nested_closures_balanced() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///workspace/nested_closures.rs", 1, "fn main() {\n    let f = |x: i32| { (x + 1) };\n    let _ = f(10);\n}\n");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_workspace_diagnostic_multiline_unclosed_brace_line_reporting() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///workspace/multiline_brace.rs", 1, "fn a() {}\n\nfn b() {\n");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    let diags = items[0]["items"].as_array().unwrap();
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0]["message"], "unclosed delimiter '{'");
    assert_eq!(diags[0]["range"]["start"]["line"], 2);
}

#[test]
fn test_workspace_diagnostic_multiple_types_mismatches_in_one_file() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///workspace/two_mismatches.rs", 1, "fn main() {\n    let x: i32 = \"str\";\n    let y: bool = 99;\n}\n");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    let diags = items[0]["items"].as_array().unwrap();
    assert_eq!(diags.len(), 2);
    assert_eq!(diags[0]["code"], "E0308");
    assert_eq!(diags[1]["code"], "E0308");
}

#[test]
fn test_workspace_diagnostic_unclosed_string_quote_coordinates() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///workspace/unclosed_quote_coords.rs", 1, "fn main() {\n    let s = \"missing end;\n}\n");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    let diags = items[0]["items"].as_array().unwrap();
    assert_eq!(diags[0]["range"]["start"]["line"], 1);
    assert_eq!(diags[0]["range"]["start"]["character"], 12);
}

#[test]
fn test_workspace_diagnostic_update_overlay_version_propagates_exact() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///workspace/ver.rs", 1, "fn foo() {}\n");
    open_test_file(&mut ov, "file:///workspace/ver.rs", 5, "fn foo() {}\n");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items[0]["version"], 5);
}

#[test]
fn test_workspace_diagnostic_clear_all_overlays_returns_empty() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///workspace/c1.rs", 1, "fn c1() {}");
    open_test_file(&mut ov, "file:///workspace/c2.rs", 1, "fn c2() {}");
    open_test_file(&mut ov, "file:///workspace/c3.rs", 1, "fn c3() {}");

    close_test_file(&mut ov, "file:///workspace/c1.rs");
    close_test_file(&mut ov, "file:///workspace/c2.rs");
    close_test_file(&mut ov, "file:///workspace/c3.rs");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 0);
}

#[test]
fn test_workspace_diagnostic_whitespace_only_file_clean() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///workspace/whitespace.rs", 1, "   \n\t\n  \r\n");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_workspace_diagnostic_negative_number_let_mismatch() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///workspace/neg.rs", 1, "fn main() {\n    let b: bool = -100;\n}\n");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    let diags = items[0]["items"].as_array().unwrap();
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0]["code"], "E0308");
    assert_eq!(diags[0]["message"], "mismatched types: expected `bool`, found `{integer}`");
}

#[test]
fn test_workspace_diagnostic_char_escapes_clean() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///workspace/escapes.rs", 1, "fn main() {\n    let _c = '\\n';\n    let _t = '\\t';\n}\n");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_workspace_diagnostic_raw_string_unclosed_hash_count() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///workspace/raw_unclosed.rs", 1, "fn main() {\n    let _s = r##\"hello#\"#;\n}\n");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    let diags = items[0]["items"].as_array().unwrap();
    assert_eq!(diags[0]["message"], "unclosed raw string literal");
}
