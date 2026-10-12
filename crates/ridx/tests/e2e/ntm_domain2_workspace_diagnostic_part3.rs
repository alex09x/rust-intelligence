/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 2: `workspace/diagnostic` pull integration tests (part 3: sequential fixes, mixed files, type checks).

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
fn test_workspace_diagnostic_char_literal_escaped_quote() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///workspace/char_quote.rs", 1, "fn foo() {\n    let _c = '\\'';\n}\n");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_workspace_diagnostic_let_bool_expected_found_integer() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///workspace/bool_int.rs", 1, "fn main() {\n    let b: bool = 42;\n}\n");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    let diags = items[0]["items"].as_array().unwrap();
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0]["code"], "E0308");
    assert_eq!(diags[0]["message"], "mismatched types: expected `bool`, found `{integer}`");
}

#[test]
fn test_workspace_diagnostic_let_str_expected_found_bool() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///workspace/str_bool.rs", 1, "fn main() {\n    let s: &str = true;\n}\n");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    let diags = items[0]["items"].as_array().unwrap();
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0]["code"], "E0308");
    assert_eq!(diags[0]["message"], "mismatched types: expected `&str`, found `bool`");
}

#[test]
fn test_workspace_diagnostic_multiple_overlays_sequential_fix() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///workspace/fix1.rs", 1, "fn foo() {");
    open_test_file(&mut ov, "file:///workspace/fix2.rs", 1, "fn bar() {");

    // Fix fix1.rs in version 2
    open_test_file(&mut ov, "file:///workspace/fix1.rs", 2, "fn foo() {}\n");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    let rep1 = items.iter().find(|i| i["uri"] == "file:///workspace/fix1.rs").unwrap();
    let rep2 = items.iter().find(|i| i["uri"] == "file:///workspace/fix2.rs").unwrap();
    assert_eq!(rep1["items"].as_array().unwrap().len(), 0);
    assert_eq!(rep1["version"], 2);
    assert_eq!(rep2["items"].as_array().unwrap().len(), 1);
}

#[test]
fn test_workspace_diagnostic_raw_string_escaped_delimiters() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///workspace/raw_delims.rs", 1, "fn main() {\n    let _s = r#\" ( [ { } ] ) \"#;\n}\n");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_workspace_diagnostic_nested_blocks_balanced() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///workspace/nested_blocks.rs", 1, "fn foo() { { { let _x = 1; } } }\n");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_workspace_diagnostic_unmatched_closing_bracket_in_workspace() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///workspace/unmatched_bracket.rs", 1, "fn foo() { ] }\n");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    let diags = items[0]["items"].as_array().unwrap();
    assert_eq!(diags.len(), 2);
    assert_eq!(diags[0]["message"], "unmatched closing delimiter ']'");
}

#[test]
fn test_workspace_diagnostic_unmatched_closing_paren_in_workspace() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///workspace/unmatched_paren.rs", 1, "fn foo() { ) }\n");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    let diags = items[0]["items"].as_array().unwrap();
    assert_eq!(diags.len(), 2);
    assert_eq!(
        diags[0]["message"],
        "mismatched closing delimiter ')', expected matching for '{' at line 1"
    );
    assert_eq!(diags[1]["message"], "unmatched closing delimiter '}'");
}

#[test]
fn test_workspace_diagnostic_five_overlays_mixed() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///workspace/f1.rs", 1, "fn f1() {}\n");
    open_test_file(&mut ov, "file:///workspace/f2.rs", 1, "fn f2() {");
    open_test_file(&mut ov, "file:///workspace/f3.rs", 1, "fn f3() {}\n");
    open_test_file(&mut ov, "file:///workspace/f4.rs", 1, "fn f4() (");
    open_test_file(&mut ov, "file:///workspace/f5.rs", 1, "fn f5() [");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 5);
}

#[test]
fn test_workspace_diagnostic_empty_file_overlay() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///workspace/empty.rs", 1, "");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["items"].as_array().unwrap().len(), 0);
}
