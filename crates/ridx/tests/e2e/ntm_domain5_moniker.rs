/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/moniker request (Domain 5: Intelligence & Completion).

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
        "id": 50,
        "method": "textDocument/moniker",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": char_col }
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
fn test_moniker_function_scheme_rust() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/moniker_fn.rs";
    open_test_file(&mut ov, uri, 1, "fn calculate_total() -> i32 { 42 }\n");

    let val = send_moniker_req(uri, 0, 3, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["scheme"], "rust");
    assert_eq!(items[0]["unique"], "project");
}

#[test]
fn test_moniker_struct_scheme_rust() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/moniker_struct.rs";
    open_test_file(&mut ov, uri, 1, "struct DataEngine {\n    val: i32,\n}\n");

    let val = send_moniker_req(uri, 0, 7, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["scheme"], "rust");
    assert_eq!(items[0]["unique"], "project");
    assert_eq!(items[0]["identifier"], "/tmp/moniker_struct.rs::DataEngine");
}

#[test]
fn test_moniker_enum_scheme_rust() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/moniker_enum.rs";
    open_test_file(&mut ov, uri, 1, "enum Status {\n    Active,\n    Inactive,\n}\n");

    let val = send_moniker_req(uri, 0, 5, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["scheme"], "rust");
    assert_eq!(items[0]["unique"], "project");
    assert_eq!(items[0]["identifier"], "/tmp/moniker_enum.rs::Status");
}

#[test]
fn test_moniker_trait_scheme_rust() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/moniker_trait.rs";
    open_test_file(&mut ov, uri, 1, "trait Greeter {\n    fn greet(&self);\n}\n");

    let val = send_moniker_req(uri, 0, 6, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["scheme"], "rust");
    assert_eq!(items[0]["unique"], "project");
    assert_eq!(items[0]["identifier"], "/tmp/moniker_trait.rs::Greeter");
}

#[test]
fn test_moniker_const_scheme_rust() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/moniker_const.rs";
    open_test_file(&mut ov, uri, 1, "const MAX_LIMIT: u32 = 1000;\n");

    let val = send_moniker_req(uri, 0, 6, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["scheme"], "rust");
    assert_eq!(items[0]["unique"], "project");
    assert_eq!(items[0]["identifier"], "/tmp/moniker_const.rs::MAX_LIMIT");
}

#[test]
fn test_moniker_unique_scope_project() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/moniker_unique.rs";
    open_test_file(&mut ov, uri, 1, "fn helper() {}\n");

    let val = send_moniker_req(uri, 0, 3, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items[0]["unique"], "project");
}

#[test]
fn test_moniker_identifier_format_file_colon_colon_name() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/format_ident.rs";
    open_test_file(&mut ov, uri, 1, "fn worker_task() {}\n");

    let val = send_moniker_req(uri, 0, 3, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    let ident = items[0]["identifier"].as_str().unwrap();
    assert!(ident.ends_with("::worker_task"));
}

#[test]
fn test_moniker_resolve_returns_params_unmodified() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 55,
        "method": "moniker/resolve",
        "params": {
            "scheme": "rust",
            "identifier": "lib::worker",
            "unique": "project"
        }
    });
    dispatch_lsp_message(LspMessage::Client(req), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
    let val = extract_response(&out);
    assert_eq!(val["id"], 55);
    assert_eq!(val["result"]["scheme"], "rust");
    assert_eq!(val["result"]["identifier"], "lib::worker");
}

#[test]
fn test_moniker_resolve_null_params_tolerance() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 56,
        "method": "moniker/resolve",
        "params": null
    });
    dispatch_lsp_message(LspMessage::Client(req), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
    let val = extract_response(&out);
    assert_eq!(val["id"], 56);
    assert_eq!(val["result"], serde_json::Value::Null);
}

#[test]
fn test_moniker_nonexistent_symbol_empty_list() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/empty_moniker.rs";
    open_test_file(&mut ov, uri, 1, "    \n");

    let val = send_moniker_req(uri, 0, 2, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 0);
}
