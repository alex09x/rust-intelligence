/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/completion request part 2 (Domain 5: Intelligence & Completion).

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

fn open_test_file(ov: &mut OverlayManager, uri: &str, version: i32, text: &str) {
    let file = ridx::lsp::protocol::uri_to_path(uri);
    ov.open_or_update(uri, &file, version, text.to_string());
}

fn send_completion_req(
    params: serde_json::Value,
    mr: &mut MultiRootWorkspace,
    ov: &mut OverlayManager,
    lc: &mut LifecycleManager,
    can: &Arc<Mutex<HashSet<serde_json::Value>>>,
) -> serde_json::Value {
    let req = json!({
        "jsonrpc": "2.0",
        "id": 100,
        "method": "textDocument/completion",
        "params": params
    });
    let mut out = Vec::new();
    dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None).unwrap();
    extract_response(&out)
}

#[test]
fn test_completion_missing_params_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let res = send_completion_req(serde_json::Value::Null, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], -32602);
}

#[test]
fn test_completion_missing_text_document_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "position": { "line": 0, "character": 0 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], -32602);
}

#[test]
fn test_completion_missing_position_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "textDocument": { "uri": "file:///path/to/main.rs" }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], -32602);
}

#[test]
fn test_completion_nonexistent_file_returns_empty_items() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "textDocument": { "uri": "file:///nonexistent/ghost_module.rs" },
        "position": { "line": 0, "character": 0 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    let items = res["result"]["items"].as_array().expect("items array");
    assert_eq!(items.len(), 0);
}

#[test]
fn test_completion_out_of_bounds_line_is_invalid_params() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_oob_test.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let x = 42;\n}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 9999, "character": 0 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], -32602, "a line past the end is invalid params (issue #2): {res}");
}

#[test]
fn test_completion_empty_document_returns_empty_items() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_empty_doc.rs";
    open_test_file(&mut ov, uri, 1, "");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 0 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    let items = res["result"]["items"].as_array().expect("items array");
    assert_eq!(items.len(), 0);
}

#[test]
fn test_completion_invoked_trigger_kind_supported() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_trigger_invoked.rs";
    open_test_file(&mut ov, uri, 1, "fn calculate(value: i32) -> i32 { value * 2 }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 12 },
        "context": { "triggerKind": 1 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    let labels: Vec<&str> = res["result"]["items"].as_array().unwrap().iter().map(|it| it["label"].as_str().unwrap()).collect();
    assert_eq!(labels, vec!["calculate"]);
}

#[test]
fn test_completion_trigger_character_dot_supported() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_trigger_dot.rs";
    open_test_file(&mut ov, uri, 1, "struct Foo;\nimpl Foo { fn bar(&self) {} }\nfn test(f: Foo) { f. }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 2, "character": 20 },
        "context": { "triggerKind": 2, "triggerCharacter": "." }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    // Without workspace index, dot receiver completion returns empty list
    assert_eq!(res["result"]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_completion_cancelled_request_returns_cancelled_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    {
        let mut guard = can.lock().unwrap();
        guard.insert(json!(100));
    }

    let params = json!({
        "textDocument": { "uri": "file:///tmp/test.rs" },
        "position": { "line": 0, "character": 0 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], -32800);
}

#[test]
fn test_completion_result_is_incomplete_flag_present() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_incomplete_flag.rs";
    open_test_file(&mut ov, uri, 1, "fn helper() {}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 5 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["isIncomplete"], false);
}
