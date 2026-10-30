/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/inlayHint filtering and edge cases (Domain 5: Intelligence & Completion).

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

    let init_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "capabilities": {}
        }
    });
    let _ = dispatch_lsp_message(LspMessage::Client(init_req), &mut out, &mut lc, &mut mr, &mut ov, &can, None);
    let init_notif = json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {}
    });
    let _ = dispatch_lsp_message(LspMessage::Client(init_notif), &mut out, &mut lc, &mut mr, &mut ov, &can, None);
    (mr, ov, lc, can)
}

fn open_test_file(ov: &mut OverlayManager, uri: &str, version: i32, text: &str) {
    let notif = json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": uri,
                "languageId": "rust",
                "version": version,
                "text": text
            }
        }
    });
    let mut mr = MultiRootWorkspace::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();
    let _ = dispatch_lsp_message(LspMessage::Client(notif), &mut out, &mut lc, &mut mr, ov, &can, None);
}

fn send_inlay_hint_req(
    params: serde_json::Value,
    mr: &mut MultiRootWorkspace,
    ov: &mut OverlayManager,
    lc: &mut LifecycleManager,
    can: &Arc<Mutex<HashSet<serde_json::Value>>>,
) -> serde_json::Value {
    let req = json!({
        "jsonrpc": "2.0",
        "id": 52,
        "method": "textDocument/inlayHint",
        "params": params
    });
    let mut out = Vec::new();
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None);
    extract_response(&out)
}

#[test]
fn test_inlay_hint_line_range_filtering_start_line_respected() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/ih_filter_start.rs";
    open_test_file(&mut ov, uri, 1, "fn line0() {}\nfn line1() {}\nfn line2() {}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "range": {
            "start": { "line": 2, "character": 0 },
            "end": { "line": 3, "character": 0 }
        }
    });
    let res = send_inlay_hint_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"], json!([]));
}

#[test]
fn test_inlay_hint_line_range_filtering_end_line_respected() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/ih_filter_end.rs";
    open_test_file(&mut ov, uri, 1, "fn line0() {}\nfn line1() {}\nfn line2() {}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "range": {
            "start": { "line": 0, "character": 0 },
            "end": { "line": 0, "character": 10 }
        }
    });
    let res = send_inlay_hint_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"], json!([]));
}

#[test]
fn test_inlay_hint_single_line_range_filtering() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/ih_single_line.rs";
    open_test_file(&mut ov, uri, 1, "fn target_line() {}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "range": {
            "start": { "line": 0, "character": 0 },
            "end": { "line": 0, "character": 19 }
        }
    });
    let res = send_inlay_hint_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"], json!([]));
}

#[test]
fn test_inlay_hint_block_comments_around_let_binding() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/ih_comments.rs";
    open_test_file(&mut ov, uri, 1, "/* block comment */ let explicit: u32 = 100;\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "range": {
            "start": { "line": 0, "character": 0 },
            "end": { "line": 0, "character": 40 }
        }
    });
    let res = send_inlay_hint_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"], json!([]));
}

#[test]
fn test_inlay_hint_cancelled_request_returns_cancelled_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let cancel_id = json!(55);
    can.lock().unwrap().insert(cancel_id.clone());

    let req = json!({
        "jsonrpc": "2.0",
        "id": cancel_id,
        "method": "textDocument/inlayHint",
        "params": {
            "textDocument": { "uri": "file:///tmp/ih_cancelled.rs" },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 1, "character": 0 }
            }
        }
    });
    let mut out = Vec::new();
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, &mut lc, &mut mr, &mut ov, &can, None);
    let res = extract_response(&out);
    assert_eq!(res["error"]["code"], -32800);
}

#[test]
fn test_inlay_hint_out_of_bounds_range_returns_empty_array() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/ih_oob.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "range": {
            "start": { "line": 100, "character": 0 },
            "end": { "line": 200, "character": 0 }
        }
    });
    let res = send_inlay_hint_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"], json!([]));
}

#[test]
fn test_inlay_hint_inverted_range_start_after_end_returns_empty() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/ih_inverted.rs";
    open_test_file(&mut ov, uri, 1, "fn f1() {}\nfn f2() {}\nfn f3() {}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "range": {
            "start": { "line": 2, "character": 0 },
            "end": { "line": 0, "character": 0 }
        }
    });
    let res = send_inlay_hint_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"], json!([]));
}

#[test]
fn test_inlay_hint_multiple_sequential_requests_stable() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/ih_sequential.rs";
    open_test_file(&mut ov, uri, 1, "fn seq() {}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "range": {
            "start": { "line": 0, "character": 0 },
            "end": { "line": 0, "character": 10 }
        }
    });
    let res1 = send_inlay_hint_req(params.clone(), &mut mr, &mut ov, &mut lc, &can);
    let res2 = send_inlay_hint_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res1["result"], res2["result"]);
    assert_eq!(res1["result"], json!([]));
}

#[test]
fn test_inlay_hint_trailing_whitespace_and_tabs_in_file() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/ih_whitespace.rs";
    open_test_file(&mut ov, uri, 1, "\t\tfn tabbed() {\t\n\t}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "range": {
            "start": { "line": 0, "character": 0 },
            "end": { "line": 1, "character": 2 }
        }
    });
    let res = send_inlay_hint_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"], json!([]));
}

#[test]
fn test_inlay_hint_large_buffer_preceding_range_handled() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/ih_large_buf.rs";
    let large_text = "fn padding() {}\n".repeat(200);
    open_test_file(&mut ov, uri, 1, &large_text);

    let params = json!({
        "textDocument": { "uri": uri },
        "range": {
            "start": { "line": 150, "character": 0 },
            "end": { "line": 155, "character": 0 }
        }
    });
    let res = send_inlay_hint_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"], json!([]));
}
