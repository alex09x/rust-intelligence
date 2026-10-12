/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 3: `textDocument/willSaveWaitUntil` request tests (part 2: response contract and reasons).

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::uri_to_path;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::Cursor;
use std::sync::{Arc, Mutex};

fn extract_response(out: &[u8]) -> Value {
    let s = String::from_utf8_lossy(out);
    for part in s.split("Content-Length: ") {
        if let Some(idx) = part.find("\r\n\r\n") {
            let body = &part[idx + 4..];
            if let Ok(v) = serde_json::from_str::<Value>(body.trim()) {
                return v;
            }
        }
    }
    Value::Null
}

fn setup_server_with_doc(uri: &str, text: &str) -> (LifecycleManager, MultiRootWorkspace, OverlayManager) {
    let mut lc = LifecycleManager::new();
    let init = json!({ "params": { "capabilities": {} } });
    let _ = lc.handle_initialize(&init);
    let mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();
    let file = uri_to_path(uri);
    om.open_with_provenance(uri, &file, 1, text.to_string(), true);
    (lc, mr, om)
}

fn dispatch_request(
    msg: Value,
    lifecycle: &mut LifecycleManager,
    multi_root: &mut MultiRootWorkspace,
    overlay_mgr: &mut OverlayManager,
) -> (bool, Value) {
    let mut out = Cursor::new(Vec::new());
    let cancelled = Arc::new(Mutex::new(HashSet::new()));
    let res = dispatch_lsp_message(
        msg,
        &mut out,
        lifecycle,
        multi_root,
        overlay_mgr,
        &cancelled,
        None,
    );
    let bytes = out.into_inner();
    (res.unwrap_or(false), extract_response(&bytes))
}

#[test]
fn test_will_save_wait_until_returns_empty_text_edits_array() {
    let uri = "file:///tmp/wait_until_empty.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");

    let (ok, resp) = dispatch_request(
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "textDocument/willSaveWaitUntil",
            "params": {
                "textDocument": { "uri": uri },
                "reason": 1
            }
        }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 1);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_will_save_wait_until_preserves_numeric_id() {
    let uri = "file:///tmp/wait_until_id.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");

    let (ok, resp) = dispatch_request(
        json!({
            "jsonrpc": "2.0",
            "id": 8888,
            "method": "textDocument/willSaveWaitUntil",
            "params": { "textDocument": { "uri": uri }, "reason": 1 }
        }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(resp["id"], 8888);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_will_save_wait_until_preserves_string_id() {
    let uri = "file:///tmp/wait_until_str_id.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");

    let (ok, resp) = dispatch_request(
        json!({
            "jsonrpc": "2.0",
            "id": "save-req-uuid-1234",
            "method": "textDocument/willSaveWaitUntil",
            "params": { "textDocument": { "uri": uri }, "reason": 1 }
        }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(resp["id"], "save-req-uuid-1234");
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_will_save_wait_until_reason_manual() {
    let uri = "file:///tmp/wait_manual.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");

    let (ok, resp) = dispatch_request(
        json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "textDocument/willSaveWaitUntil",
            "params": { "textDocument": { "uri": uri }, "reason": 1 }
        }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_will_save_wait_until_reason_after_delay() {
    let uri = "file:///tmp/wait_delay.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");

    let (ok, resp) = dispatch_request(
        json!({
            "jsonrpc": "2.0",
            "id": 20,
            "method": "textDocument/willSaveWaitUntil",
            "params": { "textDocument": { "uri": uri }, "reason": 2 }
        }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_will_save_wait_until_reason_focus_out() {
    let uri = "file:///tmp/wait_focus.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");

    let (ok, resp) = dispatch_request(
        json!({
            "jsonrpc": "2.0",
            "id": 30,
            "method": "textDocument/willSaveWaitUntil",
            "params": { "textDocument": { "uri": uri }, "reason": 3 }
        }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_will_save_wait_until_missing_reason_safe() {
    let uri = "file:///tmp/wait_no_reason.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");

    let (ok, resp) = dispatch_request(
        json!({
            "jsonrpc": "2.0",
            "id": 40,
            "method": "textDocument/willSaveWaitUntil",
            "params": { "textDocument": { "uri": uri } }
        }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_will_save_wait_until_unopened_document() {
    let mut lc = LifecycleManager::new();
    let init = json!({ "params": { "capabilities": {} } });
    let _ = lc.handle_initialize(&init);
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();
    let uri = "file:///tmp/unopened_wait.rs";

    let (ok, resp) = dispatch_request(
        json!({
            "jsonrpc": "2.0",
            "id": 50,
            "method": "textDocument/willSaveWaitUntil",
            "params": { "textDocument": { "uri": uri }, "reason": 1 }
        }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(resp["result"], json!([]));
    assert_eq!(om.open_count(), 0);
}

#[test]
fn test_will_save_wait_until_empty_params_safe() {
    let mut lc = LifecycleManager::new();
    let init = json!({ "params": { "capabilities": {} } });
    let _ = lc.handle_initialize(&init);
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();

    let (ok, resp) = dispatch_request(
        json!({
            "jsonrpc": "2.0",
            "id": 60,
            "method": "textDocument/willSaveWaitUntil",
            "params": {}
        }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_will_save_wait_until_does_not_mutate_overlay() {
    let uri = "file:///tmp/wait_mutate.rs";
    let text = "pub fn pristine() {}\n";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, text);
    let file = uri_to_path(uri);

    let (ok, resp) = dispatch_request(
        json!({
            "jsonrpc": "2.0",
            "id": 70,
            "method": "textDocument/willSaveWaitUntil",
            "params": { "textDocument": { "uri": uri }, "reason": 1 }
        }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(resp["result"], json!([]));
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, text);
    assert_eq!(overlay.version, 1);
}
