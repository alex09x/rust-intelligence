/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/completion request part 3 (Domain 5: Intelligence & Completion).

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
fn test_completion_local_let_binding_present_in_items() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_let_completion.rs";
    open_test_file(&mut ov, uri, 1, "fn demo() {\n    let counter_variable = 100;\n    cou\n}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 2, "character": 7 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    let labels: Vec<&str> = res["result"]["items"].as_array().unwrap().iter().map(|it| it["label"].as_str().unwrap()).collect();
    assert_eq!(labels, vec!["counter_variable"]);
}

#[test]
fn test_completion_function_parameter_suggested() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_param_completion.rs";
    open_test_file(&mut ov, uri, 1, "fn process_record(target_record_id: u64) {\n    tar\n}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 7 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    let labels: Vec<&str> = res["result"]["items"].as_array().unwrap().iter().map(|it| it["label"].as_str().unwrap()).collect();
    assert_eq!(labels, vec!["target_record_id"]);
}

#[test]
fn test_completion_multiple_scopes_nested_block() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_nested_scopes.rs";
    open_test_file(&mut ov, uri, 1, "fn outer() {\n    let outer_var = 1;\n    {\n        let inner_var = 2;\n        inn\n    }\n}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 4, "character": 11 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    let labels: Vec<&str> = res["result"]["items"].as_array().unwrap().iter().map(|it| it["label"].as_str().unwrap()).collect();
    assert_eq!(labels, vec!["inner_var"]);
}

#[test]
fn test_completion_overlay_update_replaces_completion_candidates() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_overlay_replace.rs";
    open_test_file(&mut ov, uri, 1, "fn test() {\n    let alpha_version = 1;\n}\n");
    // Update overlay with beta
    open_test_file(&mut ov, uri, 2, "fn test() {\n    let beta_version = 2;\n    bet\n}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 2, "character": 7 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    let labels: Vec<&str> = res["result"]["items"].as_array().unwrap().iter().map(|it| it["label"].as_str().unwrap()).collect();
    assert_eq!(labels, vec!["beta_version"]);
}

#[test]
fn test_completion_for_loop_variable_scoped() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_loop_var.rs";
    open_test_file(&mut ov, uri, 1, "fn run() {\n    for _ in 0..10 {\n        let loop_item_index = 1;\n        loop\n    }\n}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 3, "character": 12 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    let labels: Vec<&str> = res["result"]["items"].as_array().unwrap().iter().map(|it| it["label"].as_str().unwrap()).collect();
    assert_eq!(labels, vec!["loop_item_index"]);
}

#[test]
fn test_completion_while_let_binding_scoped() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_while_let.rs";
    open_test_file(&mut ov, uri, 1, "fn run(opt: Option<i32>) {\n    while let Some(_) = opt {\n        let while_elem = 1;\n        whi\n    }\n}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 3, "character": 11 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    let labels: Vec<&str> = res["result"]["items"].as_array().unwrap().iter().map(|it| it["label"].as_str().unwrap()).collect();
    assert_eq!(labels, vec!["while_elem"]);
}

#[test]
fn test_completion_if_let_binding_scoped() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_if_let.rs";
    open_test_file(&mut ov, uri, 1, "fn run(opt: Option<i32>) {\n    if let Some(_) = opt {\n        let matched_val = 1;\n        mat\n    }\n}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 3, "character": 11 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    let labels: Vec<&str> = res["result"]["items"].as_array().unwrap().iter().map(|it| it["label"].as_str().unwrap()).collect();
    assert_eq!(labels, vec!["matched_val"]);
}

#[test]
fn test_completion_match_arm_pattern_variable() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_match_pattern.rs";
    open_test_file(
        &mut ov,
        uri,
        1,
        "fn eval(val: Result<i32, String>) {\n    match val {\n        Ok(_) => {\n            let ok_result_num = 1;\n            ok_res\n        }\n        Err(_) => {}\n    }\n}\n",
    );

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 4, "character": 18 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    let labels: Vec<&str> = res["result"]["items"].as_array().unwrap().iter().map(|it| it["label"].as_str().unwrap()).collect();
    assert_eq!(labels, vec!["ok_result_num"]);
}

#[test]
fn test_completion_closure_argument_scoped() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_closure_arg.rs";
    open_test_file(&mut ov, uri, 1, "fn execute() {\n    let c = || {\n        let closure_arg = 1;\n        clos\n    };\n}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 3, "character": 12 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    let labels: Vec<&str> = res["result"]["items"].as_array().unwrap().iter().map(|it| it["label"].as_str().unwrap()).collect();
    assert_eq!(labels, vec!["closure_arg"]);
}

#[test]
fn test_completion_items_have_valid_kind_integers() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_kind_check.rs";
    open_test_file(&mut ov, uri, 1, "fn sample() {\n    let my_var = 10;\n    my\n}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 2, "character": 6 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    let items = res["result"]["items"].as_array().expect("items array");
    for item in items {
        if let Some(kind) = item["kind"].as_u64() {
            assert!(kind >= 1 && kind <= 25, "LSP CompletionItemKind must be 1..25, got {kind}");
        }
    }
}
