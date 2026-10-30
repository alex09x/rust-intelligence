/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 2: `textDocument/diagnostic` pull integration tests (part 2: whitespace, comments, literals, and CRLF).

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
fn test_document_diagnostic_empty_file_returns_zero_items() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///workspace/empty.rs";
    open_test_file(&mut ov, uri, 1, "");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["result"]["kind"], "full");
    assert_eq!(val["result"]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_document_diagnostic_only_whitespace_and_newlines_zero_items() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///workspace/whitespace.rs";
    open_test_file(&mut ov, uri, 1, "   \n\t\n  \r\n\n");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["result"]["kind"], "full");
    assert_eq!(val["result"]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_document_diagnostic_line_comment_delimiters_ignored() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///workspace/line_comments.rs";
    open_test_file(&mut ov, uri, 1, "// fn unclosed( { [ \nfn main() {}\n");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["result"]["kind"], "full");
    assert_eq!(val["result"]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_document_diagnostic_block_comment_multiline_delimiters_ignored() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///workspace/block_comments.rs";
    open_test_file(&mut ov, uri, 1, "/*\n  fn unclosed( { [ \n*/\nfn main() {}\n");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["result"]["kind"], "full");
    assert_eq!(val["result"]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_document_diagnostic_char_literal_delimiters_ignored() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///workspace/char_delims.rs";
    open_test_file(&mut ov, uri, 1, "fn foo() {\n    let _a = '{';\n    let _b = '(';\n    let _c = '[';\n}\n");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["result"]["kind"], "full");
    assert_eq!(val["result"]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_document_diagnostic_raw_string_delimiters_ignored() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///workspace/raw_string_delims.rs";
    open_test_file(&mut ov, uri, 1, "fn foo() {\n    let _s = r#\"unclosed { ( [ } ) ]\"#;\n}\n");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["result"]["kind"], "full");
    assert_eq!(val["result"]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_document_diagnostic_crlf_line_coordinates_exact() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///workspace/crlf_error.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\r\n    let x = (\r\n}\r\n");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["severity"], 1);
    assert_eq!(items[0]["source"], "ridx");
    assert_eq!(items[0]["message"], "unmatched closing delimiter '}'");
    assert_eq!(items[1]["message"], "unclosed delimiter '{'");
}

#[test]
fn test_document_diagnostic_multiline_unclosed_brace_line_exact() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///workspace/multiline_unclosed.rs";
    open_test_file(&mut ov, uri, 1, "fn a() {}\n\nfn b() {\n    let v = 1;\n");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["severity"], 1);
    assert_eq!(items[0]["source"], "ridx");
    assert_eq!(items[0]["message"], "unclosed delimiter '{'");
    assert_eq!(items[0]["range"]["start"]["line"], 2);
}

#[test]
fn test_document_diagnostic_multiple_unclosed_delimiters_count() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///workspace/two_unclosed.rs";
    open_test_file(&mut ov, uri, 1, "fn foo() {\n    let x = (;\n");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["severity"], 1);
    assert_eq!(items[1]["severity"], 1);
}

#[test]
fn test_document_diagnostic_unmatched_closing_brace_at_eof() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///workspace/extra_closing.rs";
    open_test_file(&mut ov, uri, 1, "fn foo() {}\n}");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["severity"], 1);
    assert_eq!(items[0]["source"], "ridx");
    assert_eq!(items[0]["message"], "unmatched closing delimiter '}'");
    assert_eq!(items[0]["range"]["start"]["line"], 1);
}
