/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/moniker request part 2 (Domain 5: Intelligence & Completion).

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
        "id": 60,
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
fn test_moniker_utf16_coordinates_lookup() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/utf16_moniker.rs";
    open_test_file(&mut ov, uri, 1, "// 🦀 comment with surrogate pair\nfn target_function() {}\n");

    let val = send_moniker_req(uri, 1, 3, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert!(items[0]["identifier"].as_str().unwrap().ends_with("::target_function"));
}

#[test]
fn test_moniker_out_of_bounds_line_empty() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/oob_line.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {}\n");

    let val = send_moniker_req(uri, 999, 0, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 0);
}

#[test]
fn test_moniker_out_of_bounds_column_empty() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/oob_col.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {}\n");

    let val = send_moniker_req(uri, 0, 999, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 0);
}

#[test]
fn test_moniker_overlay_definition_lookup() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/ov_def.rs";
    open_test_file(&mut ov, uri, 1, "struct Engine {\n    speed: u32,\n}\n");

    let val = send_moniker_req(uri, 0, 8, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["scheme"], "rust");
    assert_eq!(items[0]["unique"], "project");
    assert_eq!(items[0]["identifier"], "/tmp/ov_def.rs::Engine");
}

#[test]
fn test_moniker_dirty_overlay_definition_lookup() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/dirty_ov.rs";
    open_test_file(&mut ov, uri, 2, "fn new_version_fn() {}\n");

    let val = send_moniker_req(uri, 0, 4, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["scheme"], "rust");
    assert_eq!(items[0]["unique"], "project");
    assert_eq!(items[0]["identifier"], "/tmp/dirty_ov.rs::new_version_fn");
}

#[test]
fn test_moniker_empty_file_empty_monikers() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/empty.rs";
    open_test_file(&mut ov, uri, 1, "");

    let val = send_moniker_req(uri, 0, 0, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 0);
}

#[test]
fn test_moniker_multiple_distinct_symbols_in_file() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/multi_sym.rs";
    open_test_file(&mut ov, uri, 1, "fn first() {}\nfn second() {}\n");

    let val1 = send_moniker_req(uri, 0, 3, &mut mr, &mut ov, &mut lc, &can);
    let val2 = send_moniker_req(uri, 1, 3, &mut mr, &mut ov, &mut lc, &can);
    let ident1 = val1["result"][0]["identifier"].as_str().unwrap();
    let ident2 = val2["result"][0]["identifier"].as_str().unwrap();
    assert_eq!(ident1, "/tmp/multi_sym.rs::first");
    assert_eq!(ident2, "/tmp/multi_sym.rs::second");
}

#[test]
fn test_moniker_method_definition_moniker() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/method_mon.rs";
    open_test_file(&mut ov, uri, 1, "impl Worker {\n    fn execute(&self) {}\n}\n");

    let val = send_moniker_req(uri, 1, 8, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["scheme"], "rust");
    assert_eq!(items[0]["unique"], "project");
    assert_eq!(items[0]["identifier"], "/tmp/method_mon.rs::execute");
}

#[test]
fn test_moniker_module_definition_moniker() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/mod_mon.rs";
    open_test_file(&mut ov, uri, 1, "mod sub_module {\n    pub fn inner() {}\n}\n");

    let val = send_moniker_req(uri, 0, 5, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["scheme"], "rust");
    assert_eq!(items[0]["unique"], "project");
    assert_eq!(items[0]["identifier"], "/tmp/mod_mon.rs::sub_module");
}

#[test]
fn test_moniker_type_alias_moniker() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/alias_mon.rs";
    open_test_file(&mut ov, uri, 1, "type ResultNumber = Result<i32, String>;\n");

    let val = send_moniker_req(uri, 0, 6, &mut mr, &mut ov, &mut lc, &can);
    let items = val["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["scheme"], "rust");
    assert_eq!(items[0]["unique"], "project");
    assert_eq!(items[0]["identifier"], "/tmp/alias_mon.rs::ResultNumber");
}
