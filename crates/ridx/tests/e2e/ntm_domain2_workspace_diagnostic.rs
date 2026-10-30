/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspace/diagnostic pull request (Domain 2: Diagnostics & Flycheck).

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

fn send_ws_diagnostic_req(
    params: serde_json::Value,
    mr: &mut MultiRootWorkspace,
    ov: &mut OverlayManager,
    lc: &mut LifecycleManager,
    can: &Arc<Mutex<HashSet<serde_json::Value>>>,
) -> serde_json::Value {
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

fn close_test_file(ov: &mut OverlayManager, uri: &str) {
    let file = ridx::lsp::protocol::uri_to_path(uri);
    ov.close(&file);
}

#[test]
fn test_workspace_diagnostic_empty_workspace() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_ws_diagnostic_req(json!({ "previousResultIds": [] }), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["result"]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_workspace_diagnostic_single_clean_overlay() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///tmp/a.rs", 1, "fn main() {}\n");

    let val = send_ws_diagnostic_req(json!({ "previousResultIds": [] }), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["uri"], "file:///tmp/a.rs");
    assert_eq!(items[0]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_workspace_diagnostic_single_erroneous_overlay() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///tmp/b.rs", 1, "fn main() {");

    let val = send_ws_diagnostic_req(json!({ "previousResultIds": [] }), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["uri"], "file:///tmp/b.rs");
    assert_eq!(items[0]["items"].as_array().unwrap().len(), 1);
}

#[test]
fn test_workspace_diagnostic_multiple_clean_overlays() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///tmp/clean1.rs", 1, "fn foo() {}\n");
    open_test_file(&mut ov, "file:///tmp/clean2.rs", 1, "fn bar() {}\n");

    let val = send_ws_diagnostic_req(json!({ "previousResultIds": [] }), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
}

#[test]
fn test_workspace_diagnostic_multiple_erroneous_overlays() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///tmp/err1.rs", 1, "fn foo() {");
    open_test_file(&mut ov, "file:///tmp/err2.rs", 1, "fn bar() (]");

    let val = send_ws_diagnostic_req(json!({ "previousResultIds": [] }), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["items"].as_array().unwrap().len(), 1);
    assert_eq!(items[1]["items"].as_array().unwrap().len(), 1);
}

#[test]
fn test_workspace_diagnostic_mixed_clean_and_error_overlays() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///tmp/clean.rs", 1, "fn foo() {}\n");
    open_test_file(&mut ov, "file:///tmp/err.rs", 1, "fn bar() {");

    let val = send_ws_diagnostic_req(json!({ "previousResultIds": [] }), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    let clean_rep = items.iter().find(|i| i["uri"] == "file:///tmp/clean.rs").unwrap();
    let err_rep = items.iter().find(|i| i["uri"] == "file:///tmp/err.rs").unwrap();
    assert_eq!(clean_rep["items"].as_array().unwrap().len(), 0);
    assert_eq!(err_rep["items"].as_array().unwrap().len(), 1);
}

#[test]
fn test_workspace_diagnostic_overlay_version_propagated() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///tmp/v42.rs", 42, "fn foo() {}\n");

    let val = send_ws_diagnostic_req(json!({ "previousResultIds": [] }), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items[0]["version"], 42);
}

#[test]
fn test_workspace_diagnostic_deduplication_by_uri() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///tmp/dedup.rs", 1, "fn foo() {}\n");

    let val = send_ws_diagnostic_req(json!({ "previousResultIds": [] }), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
}

#[test]
fn test_workspace_diagnostic_previous_result_ids_accepted() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///tmp/f1.rs", 1, "fn foo() {}\n");

    let val = send_ws_diagnostic_req(json!({ "previousResultIds": [{ "uri": "file:///tmp/f1.rs", "value": "prev-123" }] }), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["id"], 200);
}

#[test]
fn test_workspace_diagnostic_previous_result_id_empty_unchanged() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_ws_diagnostic_req(json!({ "previousResultIds": [] }), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["result"]["items"], json!([]));
}

#[test]
fn test_workspace_diagnostic_returns_workspace_report_format() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///tmp/fmt.rs", 1, "fn main() {}\n");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["uri"], "file:///tmp/fmt.rs");
}

#[test]
fn test_workspace_diagnostic_items_array_is_present() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["result"]["items"], json!([]));
}

#[test]
fn test_workspace_diagnostic_severity_matches_document_diagnostic() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///tmp/sev.rs", 1, "fn foo() {");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    let diags = items[0]["items"].as_array().unwrap();
    assert_eq!(diags[0]["severity"], 1);
}

#[test]
fn test_workspace_diagnostic_source_matches_document_diagnostic() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///tmp/src.rs", 1, "fn foo() {");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    let diags = items[0]["items"].as_array().unwrap();
    assert_eq!(diags[0]["source"], "ridx");
}

#[test]
fn test_workspace_diagnostic_range_matches_syntax_location() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///tmp/range.rs", 1, "fn foo() {");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    let diags = items[0]["items"].as_array().unwrap();
    assert_eq!(diags[0]["range"]["start"]["line"], 0);
    assert_eq!(diags[0]["range"]["start"]["character"], 9);
}

#[test]
fn test_workspace_diagnostic_after_did_change_overlay() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///tmp/change.rs", 1, "fn foo() {");
    // Fix error in version 2
    open_test_file(&mut ov, "file:///tmp/change.rs", 2, "fn foo() {}");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items[0]["version"], 2);
    assert_eq!(items[0]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_workspace_diagnostic_after_did_close_overlay() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    open_test_file(&mut ov, "file:///tmp/close.rs", 1, "fn foo() {");
    close_test_file(&mut ov, "file:///tmp/close.rs");

    let val = send_ws_diagnostic_req(json!({}), &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 0);
}

#[test]
fn test_workspace_diagnostic_preserves_request_id() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": "ws-diag-req-777",
        "method": "workspace/diagnostic",
        "params": {}
    });
    dispatch_lsp_message(LspMessage::Client(req), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
    let val = extract_response(&out);
    assert_eq!(val["id"], "ws-diag-req-777");
}
