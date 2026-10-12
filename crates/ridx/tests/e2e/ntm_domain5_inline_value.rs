/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/inlineValue request (Domain 5: Intelligence & Completion).

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

fn send_inline_val_req(
    uri: &str,
    sl: u32,
    el: u32,
    mr: &mut MultiRootWorkspace,
    ov: &mut OverlayManager,
    lc: &mut LifecycleManager,
    can: &Arc<Mutex<HashSet<serde_json::Value>>>,
) -> serde_json::Value {
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 30,
        "method": "textDocument/inlineValue",
        "params": {
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": sl, "character": 0 },
                "end": { "line": el, "character": 100 }
            },
            "context": { "frameId": 1, "stoppedLocation": { "start": { "line": sl, "character": 0 }, "end": { "line": el, "character": 0 } } }
        }
    });
    dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None).unwrap();
    extract_response(&out)
}

fn open_test_file(ov: &mut OverlayManager, uri: &str, version: i32, text: &str) {
    let file = ridx::lsp::protocol::uri_to_path(uri);
    ov.open_or_update(uri, &file, version, text.to_string());
}

#[test]
fn test_inline_value_let_binding_variable_name() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/inline_let.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let counter = 10;\n}\n");

    let val = send_inline_val_req(uri, 1, 1, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["variableName"], "counter");
}

#[test]
fn test_inline_value_multiple_let_variables() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/inline_multi.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let x = 1;\n    let y = 2;\n}\n");

    let val = send_inline_val_req(uri, 1, 2, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["variableName"], "x");
    assert_eq!(items[1]["variableName"], "y");
}

#[test]
fn test_inline_value_function_parameter_variables() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/inline_params.rs";
    open_test_file(&mut ov, uri, 1, "fn add(alpha: i32, beta: i32) -> i32 {\n    alpha + beta\n}\n");

    let val = send_inline_val_req(uri, 0, 0, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 6);
    assert_eq!(items[1]["variableName"], "alpha");
    assert_eq!(items[3]["variableName"], "beta");
}

#[test]
fn test_inline_value_loop_variable_name() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/inline_loop.rs";
    open_test_file(&mut ov, uri, 1, "fn run() {\n    for item in items {\n    }\n}\n");

    let val = send_inline_val_req(uri, 1, 1, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["variableName"], "item");
    assert_eq!(items[1]["variableName"], "items");
}

#[test]
fn test_inline_value_rust_keywords_filtered_out() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/inline_kw.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let x = 42;\n    return x;\n}\n");

    let val = send_inline_val_req(uri, 0, 3, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["variableName"], "main");
    assert_eq!(items[1]["variableName"], "x");
    assert_eq!(items[2]["variableName"], "x");
}

#[test]
fn test_inline_value_numeric_identifiers_filtered_out() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/inline_nums.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let val = 12345;\n}\n");

    let val = send_inline_val_req(uri, 1, 1, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["variableName"], "val");
}

#[test]
fn test_inline_value_line_range_filtering_single_line() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/inline_single_line.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let a = 1;\n    let b = 2;\n    let c = 3;\n}\n");

    let val = send_inline_val_req(uri, 2, 2, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["variableName"], "b");
}

#[test]
fn test_inline_value_line_range_filtering_multi_line() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/inline_multi_line.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let a = 1;\n    let b = 2;\n    let c = 3;\n}\n");

    let val = send_inline_val_req(uri, 1, 2, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["variableName"], "a");
    assert_eq!(items[1]["variableName"], "b");
}

#[test]
fn test_inline_value_case_sensitive_lookup_flag_exact() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/inline_cs.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let myVar = 1;\n}\n");

    let val = send_inline_val_req(uri, 1, 1, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["variableName"], "myVar");
    assert_eq!(items[0]["caseSensitiveLookup"], true);
}

#[test]
fn test_inline_value_empty_range_empty_result() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/inline_empty.rs";
    open_test_file(&mut ov, uri, 1, "");

    let val = send_inline_val_req(uri, 0, 0, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 0);
}
