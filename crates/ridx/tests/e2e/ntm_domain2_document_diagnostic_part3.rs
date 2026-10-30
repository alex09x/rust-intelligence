/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 2: `textDocument/diagnostic` pull integration tests (part 3: nesting, comments, type mismatches).

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

fn send_diagnostic_req(uri: &str, mr: &mut MultiRootWorkspace, ov: &mut OverlayManager, lc: &mut LifecycleManager, can: &Arc<Mutex<HashSet<Value>>>) -> Value {
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 100,
        "method": "textDocument/diagnostic",
        "params": {
            "textDocument": { "uri": uri }
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
fn test_document_diagnostic_nested_delimiters_balanced() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///workspace/nested_balanced.rs";
    open_test_file(&mut ov, uri, 1, "fn foo() { let _v = vec![Some((1 + 2))]; }\n");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["result"]["kind"], "full");
    assert_eq!(val["result"]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_document_diagnostic_deeply_nested_unclosed_inner() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///workspace/nested_unclosed.rs";
    open_test_file(&mut ov, uri, 1, "fn foo() { let _x = ((1 + 2); }\n");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["severity"], 1);
    assert_eq!(items[0]["source"], "ridx");
    assert_eq!(items[0]["message"], "unmatched closing delimiter '}'");
    assert_eq!(items[1]["severity"], 1);
    assert_eq!(items[1]["source"], "ridx");
    assert_eq!(items[1]["message"], "unclosed delimiter '{'");
}

#[test]
fn test_document_diagnostic_nested_block_comments_with_delimiters() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///workspace/nested_comments.rs";
    open_test_file(&mut ov, uri, 1, "/* outer /* inner { [ ( */ still outer */ fn main() {}\n");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["result"]["kind"], "full");
    assert_eq!(val["result"]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_document_diagnostic_let_bool_expected_found_str() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///workspace/bool_str_mismatch.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let flag: bool = \"true\";\n}\n");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["code"], "E0308");
    assert_eq!(items[0]["severity"], 1);
    assert_eq!(items[0]["source"], "ridx");
    assert_eq!(items[0]["message"], "mismatched types: expected `bool`, found `&str`");
}

#[test]
fn test_document_diagnostic_let_i32_expected_found_bool() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///workspace/i32_bool_mismatch.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let count: i32 = false;\n}\n");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["code"], "E0308");
    assert_eq!(items[0]["severity"], 1);
    assert_eq!(items[0]["source"], "ridx");
    assert_eq!(items[0]["message"], "mismatched types: expected `i32`, found `bool`");
}

#[test]
fn test_document_diagnostic_let_str_expected_found_integer() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///workspace/str_int_mismatch.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let name: &str = 123;\n}\n");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["code"], "E0308");
    assert_eq!(items[0]["severity"], 1);
    assert_eq!(items[0]["source"], "ridx");
    assert_eq!(items[0]["message"], "mismatched types: expected `&str`, found `{integer}`");
}

#[test]
fn test_document_diagnostic_let_usize_expected_found_bool() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///workspace/usize_bool_mismatch.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let idx: usize = true;\n}\n");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["code"], "E0308");
    assert_eq!(items[0]["severity"], 1);
    assert_eq!(items[0]["source"], "ridx");
    assert_eq!(items[0]["message"], "mismatched types: expected `usize`, found `bool`");
}

#[test]
fn test_document_diagnostic_let_string_expected_found_integer() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///workspace/string_int_mismatch.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let s: String = 42;\n}\n");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["code"], "E0308");
    assert_eq!(items[0]["severity"], 1);
    assert_eq!(items[0]["source"], "ridx");
    assert_eq!(items[0]["message"], "mismatched types: expected `String`, found `{integer}`");
}

#[test]
fn test_document_diagnostic_escaped_quote_in_string() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///workspace/escaped_quote.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let _s = \"hello \\\" world ( { [\";\n}\n");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["result"]["kind"], "full");
    assert_eq!(val["result"]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_document_diagnostic_escaped_backslash_in_string() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///workspace/escaped_backslash.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let _path = \"C:\\\\path\\\\to\\\\file\";\n}\n");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["result"]["kind"], "full");
    assert_eq!(val["result"]["items"].as_array().unwrap().len(), 0);
}
