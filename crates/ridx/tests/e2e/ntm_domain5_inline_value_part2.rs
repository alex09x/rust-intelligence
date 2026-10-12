/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/inlineValue request part 2 (Domain 5: Intelligence & Completion).

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
        "id": 40,
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
fn test_inline_value_overlay_text_vs_disk_text() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/overlay_inline.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let in_memory_var = 100;\n}\n");

    let val = send_inline_val_req(uri, 1, 1, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["variableName"], "in_memory_var");
}

#[test]
fn test_inline_value_underscore_identifier_supported() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/underscore_inline.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let _unused_count = 0;\n}\n");

    let val = send_inline_val_req(uri, 1, 1, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["variableName"], "_unused_count");
}

#[test]
fn test_inline_value_utf16_column_range_exact() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/col_exact.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let var = 1;\n}\n");

    let val = send_inline_val_req(uri, 1, 1, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    let var_item = items.iter().find(|i| i["variableName"] == "var").unwrap();
    assert_eq!(var_item["range"]["start"]["line"], 1);
    assert_eq!(var_item["range"]["start"]["character"], 8);
    assert_eq!(var_item["range"]["end"]["character"], 11);
}

#[test]
fn test_inline_value_field_access_identifier_separated() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/field_access.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let val = obj.field_name;\n}\n");

    let val = send_inline_val_req(uri, 1, 1, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["variableName"], "val");
    assert_eq!(items[1]["variableName"], "obj");
    assert_eq!(items[2]["variableName"], "field_name");
}

#[test]
fn test_inline_value_method_call_identifier() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/method_call.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let res = calc.compute(arg);\n}\n");

    let val = send_inline_val_req(uri, 1, 1, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 4);
    assert_eq!(items[0]["variableName"], "res");
    assert_eq!(items[1]["variableName"], "calc");
    assert_eq!(items[2]["variableName"], "compute");
    assert_eq!(items[3]["variableName"], "arg");
}

#[test]
fn test_inline_value_shadowed_variable_inline_values() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/shadow.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let x = 1;\n    let x = 2;\n}\n");

    let val = send_inline_val_req(uri, 1, 2, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    let x_items: Vec<_> = items.iter().filter(|i| i["variableName"] == "x").collect();
    assert_eq!(x_items.len(), 2);
}

#[test]
fn test_inline_value_multiline_function_inline_values() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/multiline.rs";
    open_test_file(&mut ov, uri, 1, "fn process() {\n    let first = 10;\n    let second = 20;\n    let third = 30;\n}\n");

    let val = send_inline_val_req(uri, 0, 4, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 4);
    assert_eq!(items[0]["variableName"], "process");
    assert_eq!(items[1]["variableName"], "first");
    assert_eq!(items[2]["variableName"], "second");
    assert_eq!(items[3]["variableName"], "third");
}

#[test]
fn test_inline_value_empty_document_returns_empty() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/empty_doc.rs";
    open_test_file(&mut ov, uri, 1, "\n");

    let val = send_inline_val_req(uri, 0, 0, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 0);
}

#[test]
fn test_inline_value_nonexistent_file_empty() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/nonexistent_file_inline.rs";

    let val = send_inline_val_req(uri, 0, 10, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 0);
}

#[test]
fn test_inline_value_out_of_bounds_line_range_empty() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/short.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {}\n");

    let val = send_inline_val_req(uri, 50, 100, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 0);
}
