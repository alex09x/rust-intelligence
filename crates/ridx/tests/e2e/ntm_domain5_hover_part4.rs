/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/hover request part 4 (Domain 5: Intelligence & Completion).

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
        "id": 33,
        "method": "textDocument/hover",
        "params": params
    });
    let mut out = Vec::new();
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None);
    extract_response(&out)
}

#[test]
fn test_hover_utf16_surrogate_column_offset() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_emoji.rs";
    // 🦀 is 4 UTF-8 bytes, 2 UTF-16 code units
    open_test_file(&mut ov, uri, 1, "// 🦀 emoji\npub fn after_crab() {}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 8 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(
        res["result"]["contents"]["value"],
        "```rust\nfn after_crab()\n```"
    );
}

#[test]
fn test_hover_tab_indented_fn_hovered() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_tab.rs";
    open_test_file(&mut ov, uri, 1, "\tpub fn tab_function() {}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 10 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(
        res["result"]["contents"]["value"],
        "```rust\nfn tab_function()\n```"
    );
}

#[test]
fn test_hover_dirty_overlay_updates_definition_hover() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_dirty.rs";
    open_test_file(&mut ov, uri, 1, "pub fn initial_version() {}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 10 }
    });
    let res1 = send_hover_req(params.clone(), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res1["result"]["contents"]["value"], "```rust\nfn initial_version()\n```");

    // Update overlay with new function name at same line
    open_test_file(&mut ov, uri, 2, "pub fn updated_version() {}\n");
    let res2 = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res2["result"]["contents"]["value"], "```rust\nfn updated_version()\n```");
}

#[test]
fn test_hover_closed_document_cleared_from_overlay() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_close.rs";
    let file = ridx::lsp::protocol::uri_to_path(uri);
    ov.open_or_update(uri, &file, 1, "pub fn to_close() {}\n".to_string());

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 10 }
    });
    let res1 = send_hover_req(params.clone(), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res1["result"]["contents"]["value"], "```rust\nfn to_close()\n```");

    ov.close(&file);
    let res2 = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res2["result"].is_null());
}

#[test]
fn test_hover_multiple_sequential_hover_requests_stable() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_seq.rs";
    open_test_file(&mut ov, uri, 1, "pub struct StableTarget;\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 12 }
    });
    for _ in 0..5 {
        let res = send_hover_req(params.clone(), &mut mr, &mut ov, &mut lc, &can);
        assert_eq!(res["result"]["contents"]["value"], "```rust\nstruct StableTarget\n```");
    }
}

#[test]
fn test_hover_at_start_of_identifier() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_bound.rs";
    open_test_file(&mut ov, uri, 1, "pub fn boundary_ident() {}\n");

    // 'b' is at column 7
    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 7 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"]["contents"]["value"], "```rust\nfn boundary_ident()\n```");
}

#[test]
fn test_hover_at_end_of_identifier() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_end.rs";
    open_test_file(&mut ov, uri, 1, "pub fn boundary_ident() {}\n");

    // 't' is at column 20
    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 20 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"]["contents"]["value"], "```rust\nfn boundary_ident()\n```");
}

#[test]
fn test_hover_between_tokens_returns_null() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_space.rs";
    open_test_file(&mut ov, uri, 1, "pub fn foo() {}\n");

    // Space between 'pub' and 'fn' is at column 3
    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 3 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert!(res["result"].is_null());
}

#[test]
fn test_hover_invalid_line_negative_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "textDocument": { "uri": "file:///tmp/hover_err.rs" },
        "position": { "line": -5, "character": 0 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], -32602);
}

#[test]
fn test_hover_invalid_character_negative_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "textDocument": { "uri": "file:///tmp/hover_err.rs" },
        "position": { "line": 0, "character": -10 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], -32602);
}
