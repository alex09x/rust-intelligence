/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 3: `willSave` / `willSaveWaitUntil` tests (part 5: interleaving, empty inputs, and large documents).

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

#[test]
fn test_will_save_batch_interleaved_with_did_change() {
    let uri = "file:///tmp/interleaved_will_save.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "initial\n");
    let cancelled = Arc::new(Mutex::new(HashSet::new()));
    let file = uri_to_path(uri);

    // 1. willSave
    let mut out1 = Cursor::new(Vec::new());
    let ok1 = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/willSave",
            "params": { "textDocument": { "uri": uri }, "reason": 1 }
        }),
        &mut out1, &mut lc, &mut mr, &mut om, &cancelled, None,
    ).unwrap_or(false);
    assert_eq!(ok1, true);

    // 2. didChange
    let mut out2 = Cursor::new(Vec::new());
    let ok2 = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{ "text": "modified\n" }]
            }
        }),
        &mut out2, &mut lc, &mut mr, &mut om, &cancelled, None,
    ).unwrap_or(false);
    assert_eq!(ok2, true);

    // 3. willSave again
    let mut out3 = Cursor::new(Vec::new());
    let ok3 = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/willSave",
            "params": { "textDocument": { "uri": uri }, "reason": 1 }
        }),
        &mut out3, &mut lc, &mut mr, &mut om, &cancelled, None,
    ).unwrap_or(false);
    assert_eq!(ok3, true);

    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "modified\n");
    assert_eq!(overlay.version, 2);
}

#[test]
fn test_will_save_wait_until_interleaved_with_did_change() {
    let uri = "file:///tmp/interleaved_wait_until.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "alpha\n");
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    // 1. willSaveWaitUntil
    let mut out1 = Cursor::new(Vec::new());
    let ok1 = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "textDocument/willSaveWaitUntil",
            "params": { "textDocument": { "uri": uri }, "reason": 1 }
        }),
        &mut out1, &mut lc, &mut mr, &mut om, &cancelled, None,
    ).unwrap_or(false);
    assert_eq!(ok1, true);
    let resp1 = extract_response(&out1.into_inner());
    assert_eq!(resp1["result"], json!([]));

    // 2. didChange
    let mut out2 = Cursor::new(Vec::new());
    let ok2 = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{ "text": "beta\n" }]
            }
        }),
        &mut out2, &mut lc, &mut mr, &mut om, &cancelled, None,
    ).unwrap_or(false);
    assert_eq!(ok2, true);

    // 3. willSaveWaitUntil
    let mut out3 = Cursor::new(Vec::new());
    let ok3 = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "textDocument/willSaveWaitUntil",
            "params": { "textDocument": { "uri": uri }, "reason": 1 }
        }),
        &mut out3, &mut lc, &mut mr, &mut om, &cancelled, None,
    ).unwrap_or(false);
    assert_eq!(ok3, true);
    let resp3 = extract_response(&out3.into_inner());
    assert_eq!(resp3["result"], json!([]));
}

#[test]
fn test_will_save_empty_uri_safe() {
    let mut lc = LifecycleManager::new();
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let mut out = Cursor::new(Vec::new());
    let ok = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/willSave",
            "params": { "textDocument": { "uri": "" }, "reason": 1 }
        }),
        &mut out, &mut lc, &mut mr, &mut om, &cancelled, None,
    ).unwrap_or(false);

    assert_eq!(ok, true);
    assert_eq!(out.into_inner().len(), 0);
}

#[test]
fn test_will_save_wait_until_empty_uri_safe() {
    let mut lc = LifecycleManager::new();
    let init = json!({ "params": { "capabilities": {} } });
    let _ = lc.handle_initialize(&init);
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let mut out = Cursor::new(Vec::new());
    let ok = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "id": 100,
            "method": "textDocument/willSaveWaitUntil",
            "params": { "textDocument": { "uri": "" }, "reason": 1 }
        }),
        &mut out, &mut lc, &mut mr, &mut om, &cancelled, None,
    ).unwrap_or(false);

    assert_eq!(ok, true);
    let resp = extract_response(&out.into_inner());
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_will_save_null_params_safe() {
    let mut lc = LifecycleManager::new();
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let mut out = Cursor::new(Vec::new());
    let ok = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/willSave",
            "params": null
        }),
        &mut out, &mut lc, &mut mr, &mut om, &cancelled, None,
    ).unwrap_or(false);

    assert_eq!(ok, true);
    assert_eq!(out.into_inner().len(), 0);
}

#[test]
fn test_will_save_wait_until_null_params_safe() {
    let mut lc = LifecycleManager::new();
    let init = json!({ "params": { "capabilities": {} } });
    let _ = lc.handle_initialize(&init);
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let mut out = Cursor::new(Vec::new());
    let ok = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "id": 101,
            "method": "textDocument/willSaveWaitUntil",
            "params": null
        }),
        &mut out, &mut lc, &mut mr, &mut om, &cancelled, None,
    ).unwrap_or(false);

    assert_eq!(ok, true);
    let resp = extract_response(&out.into_inner());
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_will_save_large_document_overlay_preserved() {
    let uri = "file:///tmp/large_will_save.rs";
    let text = "pub fn line_item() {}\n".repeat(500);
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, &text);
    let file = uri_to_path(uri);
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let mut out = Cursor::new(Vec::new());
    let ok = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/willSave",
            "params": { "textDocument": { "uri": uri }, "reason": 1 }
        }),
        &mut out, &mut lc, &mut mr, &mut om, &cancelled, None,
    ).unwrap_or(false);

    assert_eq!(ok, true);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text.len(), text.len());
    assert_eq!(overlay.line_starts.len(), 501);
}

#[test]
fn test_will_save_wait_until_large_document_overlay_preserved() {
    let uri = "file:///tmp/large_wait_until.rs";
    let text = "pub fn item_large() {}\n".repeat(500);
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, &text);
    let file = uri_to_path(uri);
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let mut out = Cursor::new(Vec::new());
    let ok = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "id": 200,
            "method": "textDocument/willSaveWaitUntil",
            "params": { "textDocument": { "uri": uri }, "reason": 1 }
        }),
        &mut out, &mut lc, &mut mr, &mut om, &cancelled, None,
    ).unwrap_or(false);

    assert_eq!(ok, true);
    let resp = extract_response(&out.into_inner());
    assert_eq!(resp["result"], json!([]));
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text.len(), text.len());
}

#[test]
fn test_will_save_multi_root_routing_intact() {
    let (mut lc, mut mr, mut om) = setup_server_with_doc("file:///tmp/doc.rs", "fn f() {}\n");
    let cancelled = Arc::new(Mutex::new(HashSet::new()));
    let ws_count_before = mr.all_workspaces().count();

    let mut out = Cursor::new(Vec::new());
    let ok = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/willSave",
            "params": { "textDocument": { "uri": "file:///tmp/doc.rs" }, "reason": 1 }
        }),
        &mut out, &mut lc, &mut mr, &mut om, &cancelled, None,
    ).unwrap_or(false);

    assert_eq!(ok, true);
    assert_eq!(mr.all_workspaces().count(), ws_count_before);
}

#[test]
fn test_will_save_wait_until_preserves_multi_root_routing() {
    let (mut lc, mut mr, mut om) = setup_server_with_doc("file:///tmp/doc.rs", "fn f() {}\n");
    let cancelled = Arc::new(Mutex::new(HashSet::new()));
    let ws_count_before = mr.all_workspaces().count();

    let mut out = Cursor::new(Vec::new());
    let ok = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "id": 201,
            "method": "textDocument/willSaveWaitUntil",
            "params": { "textDocument": { "uri": "file:///tmp/doc.rs" }, "reason": 1 }
        }),
        &mut out, &mut lc, &mut mr, &mut om, &cancelled, None,
    ).unwrap_or(false);

    assert_eq!(ok, true);
    let resp = extract_response(&out.into_inner());
    assert_eq!(resp["result"], json!([]));
    assert_eq!(mr.all_workspaces().count(), ws_count_before);
}
