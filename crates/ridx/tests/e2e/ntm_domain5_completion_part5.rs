/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/completion request part 5 (Domain 5: Intelligence & Completion).

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
fn test_completion_utf16_surrogate_pair_column_indexing() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_utf16_completion.rs";
    open_test_file(
        &mut ov,
        uri,
        1,
        "// 🦀 emoji\nfn sample() {\n    let my_special_var = 1;\n    my_sp\n}\n",
    );

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 3, "character": 9 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    let labels: Vec<&str> = res["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|it| it["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, vec!["my_special_var"]);
}

#[test]
fn test_completion_in_comment_line_returns_empty_or_items() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_comment_completion.rs";
    open_test_file(
        &mut ov,
        uri,
        1,
        "// this is a regular comment\nfn foo() {}\n",
    );

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 10 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    // Position inside line comment matches no overlay symbols
    assert_eq!(res["result"]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_completion_in_string_literal_returns_valid_array() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_string_completion.rs";
    open_test_file(
        &mut ov,
        uri,
        1,
        "fn run() { let msg = \"hello world\"; }\n",
    );

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 26 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    // Position inside string literal matches no overlay symbols
    assert_eq!(res["result"]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_completion_at_exact_file_boundary_start() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_boundary_start.rs";
    open_test_file(
        &mut ov,
        uri,
        1,
        "use std::collections::HashMap;\nfn start() {}\n",
    );

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 0 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    let labels: Vec<&str> = res["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|it| it["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, vec!["start"]);
}

#[test]
fn test_completion_at_exact_file_boundary_end() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_boundary_end.rs";
    open_test_file(
        &mut ov,
        uri,
        1,
        "fn finish() {\n    let done = true;\n}\n",
    );

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 2, "character": 1 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    let labels: Vec<&str> = res["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|it| it["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, vec!["finish", "done"]);
}

#[test]
fn test_completion_data_field_for_resolve_request() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_data_field.rs";
    open_test_file(
        &mut ov,
        uri,
        1,
        "fn helper() {}\nfn main() { hel }\n",
    );

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 15 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    let items = res["result"]["items"].as_array().expect("items array");
    for item in items {
        if let Some(data) = item.get("data") {
            assert!(!data.is_null());
        }
    }
}

#[test]
fn test_completion_deprecated_item_tag_if_applicable() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_deprecated_tag.rs";
    open_test_file(
        &mut ov,
        uri,
        1,
        "#[deprecated] fn old_func() {}\nfn call() { old }\n",
    );

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 15 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    let labels: Vec<&str> = res["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|it| it["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, vec!["old_func"]);
}

#[test]
fn test_completion_whitespace_only_line() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_whitespace_line.rs";
    open_test_file(
        &mut ov,
        uri,
        1,
        "fn run() {\n    let val = 1;\n    \n}\n",
    );

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 2, "character": 4 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    let labels: Vec<&str> = res["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|it| it["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, vec!["run", "val"]);
}

#[test]
fn test_completion_multiline_raw_string_literal() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_raw_string.rs";
    open_test_file(
        &mut ov,
        uri,
        1,
        "fn text() {\n    let r = r#\"first line\nsecond line\"#;\n}\n",
    );

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 20 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    // Position inside raw string literal matches no overlay symbols
    assert_eq!(res["result"]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_completion_tab_indentation_column_handling() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_tab_indent.rs";
    open_test_file(
        &mut ov,
        uri,
        1,
        "fn tabs() {\n\tlet tab_var = 42;\n\ttab\n}\n",
    );

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 2, "character": 4 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    let labels: Vec<&str> = res["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|it| it["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, vec!["tabs", "tab_var"]);
}
