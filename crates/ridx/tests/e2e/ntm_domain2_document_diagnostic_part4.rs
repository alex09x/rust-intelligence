/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 2: `textDocument/diagnostic` pull integration tests (part 4: overlay updates, protocol invariants, multiline strings).

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
fn test_document_diagnostic_empty_params_missing_uri() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "";

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["result"]["kind"], "full");
    assert_eq!(val["result"]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_document_diagnostic_update_overlay_version_resolves_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///workspace/dynamic_fix.rs";
    open_test_file(&mut ov, uri, 1, "fn foo() {");

    let val1 = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val1["result"]["items"].as_array().unwrap().len(), 1);

    // Fix error in version 2
    open_test_file(&mut ov, uri, 2, "fn foo() {}\n");
    let val2 = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val2["result"]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_document_diagnostic_update_overlay_introduces_new_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///workspace/dynamic_break.rs";
    open_test_file(&mut ov, uri, 1, "fn foo() {}\n");

    let val1 = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val1["result"]["items"].as_array().unwrap().len(), 0);

    // Introduce error in version 2
    open_test_file(&mut ov, uri, 2, "fn foo() {\n    let x = (;\n");
    let val2 = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val2["result"]["items"].as_array().unwrap().len(), 2);
}

#[test]
fn test_document_diagnostic_multiple_let_mismatches_in_single_file() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///workspace/multiple_mismatches.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let a: i32 = \"str\";\n    let b: bool = 100;\n}\n");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["code"], "E0308");
    assert_eq!(items[1]["code"], "E0308");
    assert_eq!(items[0]["range"]["start"]["line"], 1);
    assert_eq!(items[1]["range"]["start"]["line"], 2);
}

#[test]
fn test_document_diagnostic_negative_integer_let_mismatch() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///workspace/negative_int_mismatch.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let flag: bool = -42;\n}\n");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["code"], "E0308");
    assert_eq!(items[0]["severity"], 1);
    assert_eq!(items[0]["source"], "ridx");
    assert_eq!(items[0]["message"], "mismatched types: expected `bool`, found `{integer}`");
}

#[test]
fn test_document_diagnostic_single_quote_char_escape_in_code() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///workspace/char_escapes.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let _c = '\\'';\n    let _n = '\\n';\n}\n");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["result"]["kind"], "full");
    assert_eq!(val["result"]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_document_diagnostic_raw_string_multiple_hashes_unclosed() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///workspace/unclosed_hashes.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let _s = r###\"missing end hashes##\";\n}\n");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["message"], "unclosed raw string literal");
    assert_eq!(items[1]["message"], "unclosed delimiter '{'");
}

#[test]
fn test_document_diagnostic_nested_closures_and_blocks_balanced() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///workspace/closures.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let f = |x: i32| { (x + 1) };\n    let _ = f(5);\n}\n");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["result"]["kind"], "full");
    assert_eq!(val["result"]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_document_diagnostic_response_jsonrpc_version() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///workspace/protocol_check.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {}\n");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["jsonrpc"], "2.0");
    assert_eq!(val["id"], 100);
}

#[test]
fn test_document_diagnostic_multiline_string_literal() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///workspace/multiline_string.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let _s = \"line1\nline2\nline3\";\n}\n");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["result"]["kind"], "full");
    assert_eq!(val["result"]["items"].as_array().unwrap().len(), 0);
}
