/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/inlineValue unicode, bounds, and edge cases (Domain 5: Intelligence & Completion).

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
        "id": 76,
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
fn test_inline_value_multibyte_utf8_emoji_character_col() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/iv_emoji.rs";
    open_test_file(&mut ov, uri, 1, "🚀 rocket_var\n");

    let res = send_inline_val_req(uri, 0, 0, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["variableName"], "rocket_var");
    assert_eq!(items[0]["range"]["start"]["character"], 3);
    assert_eq!(items[0]["range"]["end"]["character"], 13);
}

#[test]
fn test_inline_value_cr_lf_line_endings_supported() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/iv_crlf.rs";
    open_test_file(&mut ov, uri, 1, "win_a\r\nwin_b\r\n");

    let res = send_inline_val_req(uri, 0, 1, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["variableName"], "win_a");
    assert_eq!(items[1]["variableName"], "win_b");
}

#[test]
fn test_inline_value_comment_line_identifiers_extracted() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/iv_comment.rs";
    open_test_file(&mut ov, uri, 1, "// sample_doc_ident\n");

    let res = send_inline_val_req(uri, 0, 0, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["variableName"], "sample_doc_ident");
}

#[test]
fn test_inline_value_string_literal_embedded_alphanumeric() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/iv_string_content.rs";
    open_test_file(&mut ov, uri, 1, "\"message_payload\"\n");

    let res = send_inline_val_req(uri, 0, 0, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["variableName"], "message_payload");
}

#[test]
fn test_inline_value_zero_length_file_returns_empty() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/iv_zero.rs";
    open_test_file(&mut ov, uri, 1, "");

    let res = send_inline_val_req(uri, 0, 0, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"], json!([]));
}

#[test]
fn test_inline_value_single_char_identifier_range() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/iv_single_char.rs";
    open_test_file(&mut ov, uri, 1, "k\n");

    let res = send_inline_val_req(uri, 0, 0, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["variableName"], "k");
    assert_eq!(items[0]["range"]["start"]["character"], 0);
    assert_eq!(items[0]["range"]["end"]["character"], 1);
}

#[test]
fn test_inline_value_long_identifier_range_exact() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/iv_long.rs";
    let ident = "a".repeat(50);
    open_test_file(&mut ov, uri, 1, &format!("{ident}\n"));

    let res = send_inline_val_req(uri, 0, 0, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["variableName"], ident);
    assert_eq!(items[0]["range"]["start"]["character"], 0);
    assert_eq!(items[0]["range"]["end"]["character"], 50);
}

#[test]
fn test_inline_value_all_keywords_line_returns_empty() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/iv_keywords.rs";
    open_test_file(&mut ov, uri, 1, "pub fn struct impl for match while if else\n");

    let res = send_inline_val_req(uri, 0, 0, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 0);
}

#[test]
fn test_inline_value_response_is_json_array() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/iv_array.rs";
    open_test_file(&mut ov, uri, 1, "let test_resp = 0;\n");

    let res = send_inline_val_req(uri, 0, 0, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"], json!([
        {
            "caseSensitiveLookup": true,
            "range": {
                "start": { "line": 0, "character": 4 },
                "end": { "line": 0, "character": 13 }
            },
            "variableName": "test_resp"
        }
    ]));
}

#[test]
fn test_inline_value_end_line_before_start_line_empty() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/iv_inverted.rs";
    open_test_file(&mut ov, uri, 1, "line0_var\nline1_var\n");

    let res = send_inline_val_req(uri, 1, 0, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"], json!([]));
}
