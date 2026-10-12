/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 2: `workspace/diagnostic` pull integration tests (part 2: multi-file errors, kind, literals).

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

fn setup_initialized_server() -> (
    MultiRootWorkspace,
    OverlayManager,
    LifecycleManager,
    Arc<Mutex<HashSet<Value>>>,
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

fn send_ws_diagnostic_req(
    params: Value,
    mr: &mut MultiRootWorkspace,
    ov: &mut OverlayManager,
    lc: &mut LifecycleManager,
    can: &Arc<Mutex<HashSet<Value>>>,
) -> Value {
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

#[test]
fn test_workspace_diagnostic_three_files_all_errors() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///workspace/a.rs", 1, "fn a() {");
    open_test_file(&mut ov, "file:///workspace/b.rs", 1, "fn b() (");
    open_test_file(&mut ov, "file:///workspace/c.rs", 1, "fn c() [");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    for report in items {
        assert_eq!(report["items"].as_array().unwrap().len(), 1);
        assert_eq!(report["kind"], "full");
    }
}

#[test]
fn test_workspace_diagnostic_report_kind_is_full() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///workspace/kind_check.rs", 1, "fn foo() {}\n");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["kind"], "full");
}

#[test]
fn test_workspace_diagnostic_let_type_mismatch_in_workspace() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///workspace/mismatch.rs", 1, "fn main() {\n    let x: i32 = \"str\";\n}\n");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    let diags = items[0]["items"].as_array().unwrap();
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0]["code"], "E0308");
    assert_eq!(diags[0]["message"], "mismatched types: expected `i32`, found `&str`");
}

#[test]
fn test_workspace_diagnostic_unclosed_string_literal() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///workspace/unclosed_str.rs", 1, "fn foo() {\n    let s = \"hello;\n}\n");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    let diags = items[0]["items"].as_array().unwrap();
    assert_eq!(diags.len(), 2);
    assert_eq!(diags[0]["message"], "unclosed string literal");
}

#[test]
fn test_workspace_diagnostic_unclosed_raw_string_literal() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///workspace/unclosed_raw.rs", 1, "fn foo() {\n    let s = r#\"hello;\n}\n");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    let diags = items[0]["items"].as_array().unwrap();
    assert_eq!(diags.len(), 2);
    assert_eq!(diags[0]["message"], "unclosed raw string literal");
}

#[test]
fn test_workspace_diagnostic_multiple_errors_in_single_file() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///workspace/multi_err.rs", 1, "fn foo() {\n    let a = (\n");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    let diags = items[0]["items"].as_array().unwrap();
    assert_eq!(diags.len(), 2);
    assert_eq!(diags[0]["message"], "unclosed delimiter '('");
    assert_eq!(diags[1]["message"], "unclosed delimiter '{'");
}

#[test]
fn test_workspace_diagnostic_jsonrpc_2_0_response() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["jsonrpc"], "2.0");
    assert_eq!(val["id"], 200);
}

#[test]
fn test_workspace_diagnostic_crlf_overlay_coordinates() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///workspace/crlf.rs", 1, "fn main() {\r\n    let x = (\r\n}\r\n");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    let diags = items[0]["items"].as_array().unwrap();
    assert_eq!(diags.len(), 2);
    assert_eq!(diags[0]["source"], "ridx");
    assert_eq!(diags[0]["severity"], 1);
    assert_eq!(diags[0]["message"], "unmatched closing delimiter '}'");
    assert_eq!(diags[1]["message"], "unclosed delimiter '{'");
}

#[test]
fn test_workspace_diagnostic_block_comment_ignores_delimiters() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///workspace/block_comment.rs", 1, "/* ( { [ */\nfn main() {}\n");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_workspace_diagnostic_line_comment_ignores_delimiters() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///workspace/line_comment.rs", 1, "// ( { [\nfn main() {}\n");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["items"].as_array().unwrap().len(), 0);
}
