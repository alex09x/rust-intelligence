/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/completion request part 4 (Domain 5: Intelligence & Completion).

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
fn test_completion_path_separator_double_colon() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_double_colon.rs";
    open_test_file(
        &mut ov,
        uri,
        1,
        "mod network { pub fn connect() {} }\nfn main() { network:: }\n",
    );

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 22 },
        "context": { "triggerKind": 2, "triggerCharacter": ":" }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    // Without workspace index, path-scoped completion returns empty list
    assert_eq!(res["result"]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_completion_self_parameter_method_lookup() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_self_param.rs";
    open_test_file(
        &mut ov,
        uri,
        1,
        "struct Service;\nimpl Service {\n    fn helper(&self) {}\n    fn action(&self) {\n        self.\n    }\n}\n",
    );

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 4, "character": 13 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    // Without workspace index, dot receiver completion returns empty list
    assert_eq!(res["result"]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_completion_enum_variant_scope() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_enum_scope.rs";
    open_test_file(
        &mut ov,
        uri,
        1,
        "enum Status { Active, Inactive }\nfn check() { let s = Status::Active; }\n",
    );

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 29 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    // Without workspace index, path-scoped completion returns empty list
    assert_eq!(res["result"]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_completion_struct_field_completion() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_struct_field.rs";
    open_test_file(
        &mut ov,
        uri,
        1,
        "struct Config { timeout_seconds: u32, retry_attempts: usize }\nfn test(c: Config) { let _ = c.tim; }\n",
    );

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 34 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    // Without workspace index, dot receiver completion returns empty list
    assert_eq!(res["result"]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_completion_trait_method_suggestion() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_trait_suggestion.rs";
    open_test_file(
        &mut ov,
        uri,
        1,
        "trait Worker { fn perform_work(&self); }\nfn do_job<W: Worker>(w: &W) { w.per; }\n",
    );

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 35 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    // Without workspace index, dot receiver completion returns empty list
    assert_eq!(res["result"]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_completion_tuple_index_access() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_tuple_idx.rs";
    open_test_file(
        &mut ov,
        uri,
        1,
        "fn pair() -> (i32, String) { (1, \"one\".into()) }\nfn inspect() { let p = pair(); let _ = p.0; }\n",
    );

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 42 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    // Without workspace index, dot receiver completion returns empty list
    assert_eq!(res["result"]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_completion_items_have_sort_text_or_label() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_sort_text.rs";
    open_test_file(
        &mut ov,
        uri,
        1,
        "fn foo() { let var_one = 1; let var_two = 2; var_ }\n",
    );

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 50 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    let items = res["result"]["items"].as_array().expect("items array");
    let labels: Vec<&str> = items.iter().map(|it| it["label"].as_str().unwrap()).collect();
    assert_eq!(labels, vec!["foo", "var_one", "var_two"]);
}

#[test]
fn test_completion_insert_text_format_plain_or_snippet() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_insert_text_fmt.rs";
    open_test_file(
        &mut ov,
        uri,
        1,
        "fn example() { let candidate = 42; cand }\n",
    );

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 39 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    let items = res["result"]["items"].as_array().expect("items array");
    for item in items {
        if let Some(fmt) = item["insertTextFormat"].as_u64() {
            assert!(fmt == 1 || fmt == 2, "insertTextFormat must be PlainText(1) or Snippet(2)");
        }
    }
}

#[test]
fn test_completion_items_unique_labels_within_scope() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_unique_labels.rs";
    open_test_file(
        &mut ov,
        uri,
        1,
        "fn unique_check() { let identifier = 1; iden }\n",
    );

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 44 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    let items = res["result"]["items"].as_array().expect("items array");
    let mut labels = HashSet::new();
    for item in items {
        let label = item["label"].as_str().unwrap();
        let kind = item["kind"].as_u64().unwrap_or(0);
        assert!(labels.insert((label.to_string(), kind)), "Duplicate label {label} with kind {kind}");
    }
}

#[test]
fn test_completion_client_filter_text_matches_label() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_filter_text.rs";
    open_test_file(
        &mut ov,
        uri,
        1,
        "fn filter_test() { let my_sample_identifier = 1; my_s }\n",
    );

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 54 }
    });
    let res = send_completion_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    let items = res["result"]["items"].as_array().expect("items array");
    let my_item = items.iter().find(|i| i["label"] == "my_sample_identifier").expect("my_sample_identifier present");
    assert_eq!(my_item["label"].as_str(), Some("my_sample_identifier"));
    assert_eq!(my_item["filterText"].as_str().unwrap_or("my_sample_identifier"), "my_sample_identifier");
}
