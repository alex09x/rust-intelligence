/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/hover request part 5 (Domain 5: Intelligence & Completion).

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

fn send_hover_req(
    params: serde_json::Value,
    mr: &mut MultiRootWorkspace,
    ov: &mut OverlayManager,
    lc: &mut LifecycleManager,
    can: &Arc<Mutex<HashSet<serde_json::Value>>>,
) -> serde_json::Value {
    let req = json!({
        "jsonrpc": "2.0",
        "id": 34,
        "method": "textDocument/hover",
        "params": params
    });
    let mut out = Vec::new();
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None);
    extract_response(&out)
}

#[test]
fn test_hover_nested_module_item() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_nested.rs";
    open_test_file(&mut ov, uri, 1, "mod inner {\n    pub fn inner_work() {}\n}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 12 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(
        res["result"]["contents"]["value"],
        "```rust\nfn inner_work()\n```"
    );
}

#[test]
fn test_hover_struct_with_generics() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_gen_struct.rs";
    open_test_file(&mut ov, uri, 1, "pub struct GenericBox<T> {\n    pub item: T,\n}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 14 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(
        res["result"]["contents"]["value"],
        "```rust\nstruct GenericBox\n```"
    );
}

#[test]
fn test_hover_enum_with_tuple_variants() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_enum_tuple.rs";
    open_test_file(&mut ov, uri, 1, "pub enum EventKind {\n    Click(i32, i32),\n    Key,\n}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 12 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(
        res["result"]["contents"]["value"],
        "```rust\nenum EventKind\n```"
    );
}

#[test]
fn test_hover_const_with_explicit_type() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_const_type.rs";
    open_test_file(&mut ov, uri, 1, "pub const MAX_PAYLOAD_SIZE: usize = 65536;\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 15 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(
        res["result"]["contents"]["value"],
        "```rust\nconst MAX_PAYLOAD_SIZE\n```"
    );
}

#[test]
fn test_hover_static_raw_definition() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_static_raw.rs";
    open_test_file(&mut ov, uri, 1, "pub static GLOBAL_FLAG: bool = false;\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 15 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(
        res["result"]["contents"]["value"],
        "```rust\nstatic GLOBAL_FLAG\n```"
    );
}
