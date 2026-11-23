/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for moniker unicode offsets, scope resolution, and resolve payloads (Domain 5: Intelligence & Completion).

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
        "id": 88,
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
        "id": 89,
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
fn test_moniker_multibyte_utf8_character_col_offset() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/m_utf8.rs";
    open_test_file(&mut ov, uri, 1, "// 🚀\nfn space_mission() {}\n");

    let res = send_moniker_req(uri, 1, 4, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["identifier"], "/tmp/m_utf8.rs::space_mission");
}

#[test]
fn test_moniker_tab_indented_symbol_definition() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/m_tab.rs";
    open_test_file(&mut ov, uri, 1, "\tfn tabbed_task() {}\n");

    let res = send_moniker_req(uri, 0, 5, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["identifier"], "/tmp/m_tab.rs::tabbed_task");
}

#[test]
fn test_moniker_cr_lf_line_endings_supported() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/m_crlf.rs";
    open_test_file(&mut ov, uri, 1, "fn win_fn() {}\r\n");

    let res = send_moniker_req(uri, 0, 4, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["identifier"], "/tmp/m_crlf.rs::win_fn");
}

#[test]
fn test_moniker_response_is_json_array() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/m_arr.rs";
    open_test_file(&mut ov, uri, 1, "fn dummy() {}\n");

    let res = send_moniker_req(uri, 0, 3, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(
        res["result"],
        json!([
            {
                "identifier": "/tmp/m_arr.rs::dummy",
                "scheme": "rust",
                "unique": "project"
            }
        ])
    );
}

#[test]
fn test_moniker_zero_length_file_returns_empty() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/m_zero.rs";
    open_test_file(&mut ov, uri, 1, "");

    let res = send_moniker_req(uri, 0, 0, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"], json!([]));
}

#[test]
fn test_moniker_negative_position_handled_gracefully() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/m_neg.rs";
    open_test_file(&mut ov, uri, 1, "fn nop() {}\n");

    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 90,
        "method": "textDocument/moniker",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": 999, "character": 999 }
        }
    });
    dispatch_lsp_message(LspMessage::Client(req), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
    let res = extract_response(&out);
    assert_eq!(res["error"]["code"], -32602, "a line past the end is invalid params (issue #2): {res}");
}

#[test]
fn test_moniker_associated_function_moniker() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/m_assoc.rs";
    open_test_file(&mut ov, uri, 1, "struct Worker;\nfn execute_work() {}\n");

    let res = send_moniker_req(uri, 1, 4, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["identifier"], "/tmp/m_assoc.rs::execute_work");
}

#[test]
fn test_moniker_fn_in_nested_scope() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/m_nested.rs";
    open_test_file(&mut ov, uri, 1, "fn outer() {\n    fn inner_helper() {}\n}\n");

    let res = send_moniker_req(uri, 1, 8, &mut mr, &mut ov, &mut lc, &can);
    let items = res["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["identifier"], "/tmp/m_nested.rs::inner_helper");
}

#[test]
fn test_moniker_repeated_name_in_different_files() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri1 = "file:///tmp/m_f1.rs";
    let uri2 = "file:///tmp/m_f2.rs";
    open_test_file(&mut ov, uri1, 1, "fn common_name() {}\n");
    open_test_file(&mut ov, uri2, 1, "fn common_name() {}\n");

    let res1 = send_moniker_req(uri1, 0, 4, &mut mr, &mut ov, &mut lc, &can);
    let res2 = send_moniker_req(uri2, 0, 4, &mut mr, &mut ov, &mut lc, &can);

    let id1 = res1["result"][0]["identifier"].as_str().unwrap();
    let id2 = res2["result"][0]["identifier"].as_str().unwrap();
    assert_eq!(id1, "/tmp/m_f1.rs::common_name");
    assert_eq!(id2, "/tmp/m_f2.rs::common_name");
}

#[test]
fn test_moniker_resolve_array_params_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!([
        { "scheme": "rust", "identifier": "mod1::a" },
        { "scheme": "rust", "identifier": "mod2::b" }
    ]);
    let res = send_moniker_resolve_req(params.clone(), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"], params);
}
