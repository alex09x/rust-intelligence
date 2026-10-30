/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for moniker/resolve and moniker overlay updates (Domain 5: Intelligence & Completion).

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

fn send_moniker_req(
    uri: &str,
    line: u32,
    char_col: u32,
    mr: &mut MultiRootWorkspace,
    ov: &mut OverlayManager,
    lc: &mut LifecycleManager,
    can: &Arc<Mutex<HashSet<serde_json::Value>>>,
) -> serde_json::Value {
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 86,
        "method": "textDocument/moniker",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": char_col }
        }
    });
    dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None).unwrap();
    extract_response(&out)
}

fn send_moniker_resolve_req(
    params: serde_json::Value,
    mr: &mut MultiRootWorkspace,
    ov: &mut OverlayManager,
    lc: &mut LifecycleManager,
    can: &Arc<Mutex<HashSet<serde_json::Value>>>,
) -> serde_json::Value {
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 87,
        "method": "moniker/resolve",
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
fn test_moniker_dirty_overlay_updates_identifier() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/m_dirty.rs";
    open_test_file(&mut ov, uri, 1, "fn initial_name() {}\n");
    open_test_file(&mut ov, uri, 2, "fn updated_name() {}\n");

    let res = send_moniker_req(uri, 0, 4, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["identifier"], "/tmp/m_dirty.rs::updated_name");
}

#[test]
fn test_moniker_multiple_sequential_requests_stable() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/m_seq.rs";
    open_test_file(&mut ov, uri, 1, "struct PersistentItem {}\n");

    let res1 = send_moniker_req(uri, 0, 8, &mut mr, &mut ov, &mut lc, &can);
    let res2 = send_moniker_req(uri, 0, 8, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res1["result"], res2["result"]);
}

#[test]
fn test_moniker_resolve_custom_kind_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "scheme": "rust",
        "identifier": "crate::mod::Item",
        "unique": "project",
        "kind": "export"
    });
    let res = send_moniker_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"]["kind"], "export");
}

#[test]
fn test_moniker_resolve_group_field_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "scheme": "rust",
        "identifier": "core::slice",
        "unique": "project",
        "group": "std"
    });
    let res = send_moniker_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"]["group"], "std");
}

#[test]
fn test_moniker_resolve_data_payload_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "scheme": "rust",
        "identifier": "alloc::vec",
        "unique": "project",
        "data": { "version": "1.0", "repo": "upstream" }
    });
    let res = send_moniker_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"]["data"]["version"], "1.0");
    assert_eq!(res["result"]["data"]["repo"], "upstream");
}

#[test]
fn test_moniker_resolve_empty_object_params() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({});
    let res = send_moniker_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"], json!({}));
}

#[test]
fn test_moniker_resolve_string_params_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!("simple_string_payload");
    let res = send_moniker_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"], "simple_string_payload");
}

#[test]
fn test_moniker_resolve_numeric_id_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({ "id": 12345 });
    let res = send_moniker_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"]["id"], 12345);
}

#[test]
fn test_moniker_resolve_idempotent_multiple_calls() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "scheme": "rust",
        "identifier": "std::collections::HashMap",
        "unique": "project"
    });
    let res1 = send_moniker_resolve_req(params.clone(), &mut mr, &mut ov, &mut lc, &can);
    let res2 = send_moniker_resolve_req(res1["result"].clone(), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res1["result"], res2["result"]);
}

#[test]
fn test_moniker_whitespace_surrounding_position_empty() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/m_ws.rs";
    open_test_file(&mut ov, uri, 1, "    fn ws_fn() {}    \n");

    let res = send_moniker_req(uri, 0, 1, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 0);
}
