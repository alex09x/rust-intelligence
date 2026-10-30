/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/hover request part 2 (Domain 5: Intelligence & Completion).

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

fn send_hover_req(
    params: serde_json::Value,
    mr: &mut MultiRootWorkspace,
    ov: &mut OverlayManager,
    lc: &mut LifecycleManager,
    can: &Arc<Mutex<HashSet<serde_json::Value>>>,
) -> serde_json::Value {
    let req = json!({
        "jsonrpc": "2.0",
        "id": 31,
        "method": "textDocument/hover",
        "params": params
    });
    let mut out = Vec::new();
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None);
    extract_response(&out)
}

#[test]
fn test_hover_whitespace_character_returns_null() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_whitespace.rs";
    open_test_file(&mut ov, uri, 1, "    \n    fn test() {}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 2 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert!(res["result"].is_null());
}

#[test]
fn test_hover_comment_line_returns_null() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_comment.rs";
    open_test_file(&mut ov, uri, 1, "// This is a documentation note\npub fn work() {}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 5 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert!(res["result"].is_null());
}

#[test]
fn test_hover_out_of_bounds_line_returns_null() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_oob_line.rs";
    open_test_file(&mut ov, uri, 1, "pub fn single() {}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 9999, "character": 0 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert!(res["result"].is_null());
}

#[test]
fn test_hover_out_of_bounds_character_returns_null() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_oob_char.rs";
    open_test_file(&mut ov, uri, 1, "pub fn short() {}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 500 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert!(res["result"].is_null());
}

#[test]
fn test_hover_cancelled_request_returns_cancelled_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    {
        let mut guard = can.lock().unwrap();
        guard.insert(json!(31));
    }
    let params = json!({
        "textDocument": { "uri": "file:///tmp/cancelled.rs" },
        "position": { "line": 0, "character": 0 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], -32800);
}

#[test]
fn test_hover_markdown_kind_is_explicit() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_kind.rs";
    open_test_file(&mut ov, uri, 1, "pub fn target_function() -> bool { true }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 10 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"]["contents"]["kind"], "markdown");
}

#[test]
fn test_hover_codeblock_fencing_present() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_fence.rs";
    open_test_file(&mut ov, uri, 1, "pub fn fenced_item() {}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 10 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    let val = res["result"]["contents"]["value"].as_str().expect("string value");
    assert_eq!(&val[..7], "```rust");
    assert_eq!(&val[val.len() - 3..], "```");
}

#[test]
fn test_hover_type_alias_shows_type_definition() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_alias.rs";
    open_test_file(&mut ov, uri, 1, "pub type MetricScore = f64;\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 12 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["contents"]["kind"], "markdown");
    assert_eq!(res["result"]["contents"]["value"], "```rust\ntype MetricScore\n```");
}

#[test]
fn test_hover_module_definition_shows_mod() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_mod.rs";
    open_test_file(&mut ov, uri, 1, "pub mod engine;\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 10 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["contents"]["kind"], "markdown");
    assert_eq!(res["result"]["contents"]["value"], "```rust\nmod engine\n```");
}

#[test]
fn test_hover_union_definition_shows_union() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_union.rs";
    open_test_file(&mut ov, uri, 1, "pub union RawPayload {\n    pub num: u64,\n}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 12 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["contents"]["kind"], "markdown");
    assert_eq!(res["result"]["contents"]["value"], "```rust\nunion RawPayload\n```");
}
