/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/inlineValue expressions and stability (Domain 5: Intelligence & Completion).

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
        "id": 75,
        "method": "textDocument/inlineValue",
        "params": {
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": sl, "character": 0 },
                "end": { "line": el, "character": 100 }
            }
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
fn test_inline_value_const_and_static_identifiers() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/iv_const_static.rs";
    open_test_file(&mut ov, uri, 1, "const MAX_LIMIT: usize = 1024;\nstatic GLOBAL_FLAG: bool = true;\n");

    let res = send_inline_val_req(uri, 0, 1, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 4);
    assert_eq!(items[0]["variableName"], "MAX_LIMIT");
    assert_eq!(items[1]["variableName"], "usize");
    assert_eq!(items[2]["variableName"], "GLOBAL_FLAG");
    assert_eq!(items[3]["variableName"], "bool");
}

#[test]
fn test_inline_value_nested_block_scopes_identifiers() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/iv_nested_blocks.rs";
    open_test_file(&mut ov, uri, 1, "fn run() {\n    let outer = 1;\n    {\n        let inner = outer + 2;\n    }\n}\n");

    let res = send_inline_val_req(uri, 3, 3, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["variableName"], "inner");
    assert_eq!(items[1]["variableName"], "outer");
}

#[test]
fn test_inline_value_macro_call_identifiers() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/iv_macro.rs";
    open_test_file(&mut ov, uri, 1, "println!(log_message);\n");

    let res = send_inline_val_req(uri, 0, 0, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["variableName"], "println");
    assert_eq!(items[1]["variableName"], "log_message");
}

#[test]
fn test_inline_value_array_indexing_expression_identifiers() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/iv_indexing.rs";
    open_test_file(&mut ov, uri, 1, "let elem = buffer[cursor_idx];\n");

    let res = send_inline_val_req(uri, 0, 0, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["variableName"], "elem");
    assert_eq!(items[1]["variableName"], "buffer");
    assert_eq!(items[2]["variableName"], "cursor_idx");
}

#[test]
fn test_inline_value_binary_arithmetic_expression_identifiers() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/iv_arithmetic.rs";
    open_test_file(&mut ov, uri, 1, "total_sum = base_val + delta_val * scale_factor;\n");

    let res = send_inline_val_req(uri, 0, 0, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 4);
    assert_eq!(items[0]["variableName"], "total_sum");
    assert_eq!(items[1]["variableName"], "base_val");
    assert_eq!(items[2]["variableName"], "delta_val");
    assert_eq!(items[3]["variableName"], "scale_factor");
}

#[test]
fn test_inline_value_generic_type_arguments_identifiers() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/iv_generics.rs";
    open_test_file(&mut ov, uri, 1, "let map: HashMap<KeyType, ValueType>;\n");

    let res = send_inline_val_req(uri, 0, 0, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 4);
    assert_eq!(items[0]["variableName"], "map");
    assert_eq!(items[1]["variableName"], "HashMap");
    assert_eq!(items[2]["variableName"], "KeyType");
    assert_eq!(items[3]["variableName"], "ValueType");
}

#[test]
fn test_inline_value_multiple_sequential_requests_stable() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/iv_stable.rs";
    open_test_file(&mut ov, uri, 1, "let stable_x = 42;\n");

    let res1 = send_inline_val_req(uri, 0, 0, &mut mr, &mut ov, &mut lc, &can);
    let res2 = send_inline_val_req(uri, 0, 0, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res1["result"], res2["result"]);
}

#[test]
fn test_inline_value_trailing_semicolon_and_commas() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/iv_punct.rs";
    open_test_file(&mut ov, uri, 1, "foo, bar, baz;\n");

    let res = send_inline_val_req(uri, 0, 0, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["variableName"], "foo");
    assert_eq!(items[1]["variableName"], "bar");
    assert_eq!(items[2]["variableName"], "baz");
}

#[test]
fn test_inline_value_whitespace_indentation_start_col() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/iv_indent.rs";
    open_test_file(&mut ov, uri, 1, "    indented_var\n");

    let res = send_inline_val_req(uri, 0, 0, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["variableName"], "indented_var");
    assert_eq!(items[0]["range"]["start"]["character"], 4);
    assert_eq!(items[0]["range"]["end"]["character"], 16);
}

#[test]
fn test_inline_value_tab_indentation_character_positions() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/iv_tab_indent.rs";
    open_test_file(&mut ov, uri, 1, "\ttab_var\n");

    let res = send_inline_val_req(uri, 0, 0, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["variableName"], "tab_var");
    assert_eq!(items[0]["range"]["start"]["character"], 1);
    assert_eq!(items[0]["range"]["end"]["character"], 8);
}
