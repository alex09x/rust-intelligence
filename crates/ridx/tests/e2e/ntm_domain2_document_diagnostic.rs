/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/diagnostic pull request (Domain 2: Diagnostics & Flycheck).

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

fn send_diagnostic_req(
    uri: &str,
    mr: &mut MultiRootWorkspace,
    ov: &mut OverlayManager,
    lc: &mut LifecycleManager,
    can: &Arc<Mutex<HashSet<serde_json::Value>>>,
) -> serde_json::Value {
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
fn test_document_diagnostic_clean_file_empty_items() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/clean.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let x = 42;\n}\n");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["result"]["kind"], "full");
    assert_eq!(val["result"]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_document_diagnostic_unclosed_brace() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/unclosed_brace.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["severity"], 1);
    assert_eq!(items[0]["source"], "ridx");
    assert_eq!(items[0]["message"], "unclosed delimiter '{'");
}

#[test]
fn test_document_diagnostic_unclosed_paren() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/unclosed_paren.rs";
    open_test_file(&mut ov, uri, 1, "fn foo(x: i32");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["severity"], 1);
    assert_eq!(items[0]["source"], "ridx");
    assert_eq!(items[0]["message"], "unclosed delimiter '('");
}

#[test]
fn test_document_diagnostic_unclosed_bracket() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/unclosed_bracket.rs";
    open_test_file(&mut ov, uri, 1, "fn foo() { let arr = [1, 2; }");

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
fn test_document_diagnostic_mismatched_paren_bracket() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/mismatch_pb.rs";
    open_test_file(&mut ov, uri, 1, "fn foo() { (] }");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["severity"], 1);
    assert_eq!(items[0]["source"], "ridx");
    assert_eq!(items[0]["message"], "unmatched closing delimiter ']'");
    assert_eq!(items[0]["range"], json!({ "start": { "line": 0, "character": 12 }, "end": { "line": 0, "character": 13 } }));
}

#[test]
fn test_document_diagnostic_mismatched_paren_brace() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/mismatch_pbr.rs";
    open_test_file(&mut ov, uri, 1, "fn foo() { (} )");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["message"], "unmatched closing delimiter '}'");
    assert_eq!(items[1]["message"], "mismatched closing delimiter ')', expected matching for '{' at line 1");
}

#[test]
fn test_document_diagnostic_unmatched_closing_paren() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/unmatched_paren.rs";
    open_test_file(&mut ov, uri, 1, "fn foo() {} )");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["severity"], 1);
    assert_eq!(items[0]["source"], "ridx");
    assert_eq!(items[0]["message"], "unmatched closing delimiter ')'");
    assert_eq!(items[0]["range"], json!({ "start": { "line": 0, "character": 12 }, "end": { "line": 0, "character": 13 } }));
}

#[test]
fn test_document_diagnostic_unmatched_closing_brace() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/unmatched_brace.rs";
    open_test_file(&mut ov, uri, 1, "fn foo() {} }");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["severity"], 1);
    assert_eq!(items[0]["source"], "ridx");
    assert_eq!(items[0]["message"], "unmatched closing delimiter '}'");
    assert_eq!(items[0]["range"], json!({ "start": { "line": 0, "character": 12 }, "end": { "line": 0, "character": 13 } }));
}

#[test]
fn test_document_diagnostic_unmatched_closing_bracket() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/unmatched_bracket.rs";
    open_test_file(&mut ov, uri, 1, "fn foo() { ] }");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["message"], "unmatched closing delimiter ']'");
    assert_eq!(items[1]["message"], "unmatched closing delimiter '}'");
}

#[test]
fn test_document_diagnostic_unclosed_string_literal() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/unclosed_str.rs";
    open_test_file(&mut ov, uri, 1, "fn foo() {\n    let s = \"hello;\n}\n");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["message"], "unclosed string literal");
    assert_eq!(items[1]["message"], "unclosed delimiter '{'");
}

#[test]
fn test_document_diagnostic_unclosed_raw_string_literal() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/unclosed_raw_str.rs";
    open_test_file(&mut ov, uri, 1, "fn foo() {\n    let s = r#\"hello;\n}\n");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["message"], "unclosed raw string literal");
    assert_eq!(items[1]["message"], "unclosed delimiter '{'");
}

#[test]
fn test_document_diagnostic_comments_delimiters_ignored() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/comment_delim.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    // ( [ {\n    /* ) ] } */\n}\n");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 0);
}

#[test]
fn test_document_diagnostic_string_delimiters_ignored() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/string_delim.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let s = \"( [ { ) ] }\";\n}\n");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 0);
}

#[test]
fn test_document_diagnostic_let_type_mismatch_e0308() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/type_mismatch.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let x: i32 = \"string\";\n}\n");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["code"], "E0308");
    assert_eq!(items[0]["severity"], 1);
    assert_eq!(items[0]["source"], "ridx");
    assert_eq!(
        items[0]["range"],
        json!({
            "start": { "line": 1, "character": 17 },
            "end": { "line": 1, "character": 25 }
        })
    );
}

#[test]
fn test_document_diagnostic_overlay_precedence_over_disk() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/overlay_precedence.rs";
    // Overlay has unclosed brace
    open_test_file(&mut ov, uri, 2, "fn main() {");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["severity"], 1);
    assert_eq!(items[0]["source"], "ridx");
    assert_eq!(items[0]["message"], "unclosed delimiter '{'");
}

#[test]
fn test_document_diagnostic_nonexistent_uri_empty_items() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/nonexistent_file_definitely_missing.rs";

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["result"]["kind"], "full");
    assert_eq!(val["result"]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_document_diagnostic_severity_is_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/severity.rs";
    open_test_file(&mut ov, uri, 1, "fn foo() {");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items[0]["severity"], 1);
}

#[test]
fn test_document_diagnostic_source_is_ridx() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/source.rs";
    open_test_file(&mut ov, uri, 1, "fn foo() {");

    let val = send_diagnostic_req(uri, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items[0]["source"], "ridx");
}
