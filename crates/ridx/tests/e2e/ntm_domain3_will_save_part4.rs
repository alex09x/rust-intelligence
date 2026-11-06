/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 3: `willSave` / `willSaveWaitUntil` tests (part 4: URI formats and client extensions).

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
fn test_will_save_with_windows_uri() {
    let uri = "file:///C:/workspace/main.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn win() {}\n");
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let mut out = Cursor::new(Vec::new());
    let ok = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/willSave",
            "params": { "textDocument": { "uri": uri }, "reason": 1 }
        }),
        &mut out,
        &mut lc,
        &mut mr,
        &mut om,
        &cancelled,
        None,
    )
    .unwrap_or(false);

    assert!(ok);
    assert_eq!(out.into_inner().len(), 0);
}

#[test]
fn test_will_save_wait_until_with_windows_uri() {
    let uri = "file:///C:/workspace/main.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn win() {}\n");
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let mut out = Cursor::new(Vec::new());
    let ok = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "textDocument/willSaveWaitUntil",
            "params": { "textDocument": { "uri": uri }, "reason": 1 }
        }),
        &mut out,
        &mut lc,
        &mut mr,
        &mut om,
        &cancelled,
        None,
    )
    .unwrap_or(false);

    assert!(ok);
    let resp = extract_response(&out.into_inner());
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_will_save_with_url_encoded_space_uri() {
    let uri = "file:///tmp/my%20dir/lib.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn spaced() {}\n");
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let mut out = Cursor::new(Vec::new());
    let ok = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/willSave",
            "params": { "textDocument": { "uri": uri }, "reason": 1 }
        }),
        &mut out,
        &mut lc,
        &mut mr,
        &mut om,
        &cancelled,
        None,
    )
    .unwrap_or(false);

    assert!(ok);
    assert_eq!(out.into_inner().len(), 0);
}

#[test]
fn test_will_save_wait_until_with_url_encoded_space_uri() {
    let uri = "file:///tmp/my%20dir/lib.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn spaced() {}\n");
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let mut out = Cursor::new(Vec::new());
    let ok = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "textDocument/willSaveWaitUntil",
            "params": { "textDocument": { "uri": uri }, "reason": 1 }
        }),
        &mut out,
        &mut lc,
        &mut mr,
        &mut om,
        &cancelled,
        None,
    )
    .unwrap_or(false);

    assert!(ok);
    let resp = extract_response(&out.into_inner());
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_will_save_on_closed_document_remains_closed() {
    let uri = "file:///tmp/closed_doc.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn c() {}\n");
    let file = uri_to_path(uri);
    om.close_document(&file);
    assert_eq!(om.open_count(), 0);
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let mut out = Cursor::new(Vec::new());
    let ok = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/willSave",
            "params": { "textDocument": { "uri": uri }, "reason": 1 }
        }),
        &mut out,
        &mut lc,
        &mut mr,
        &mut om,
        &cancelled,
        None,
    )
    .unwrap_or(false);

    assert!(ok);
    assert_eq!(om.open_count(), 0);
}

#[test]
fn test_will_save_wait_until_on_closed_document() {
    let uri = "file:///tmp/closed_wait.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn c() {}\n");
    let file = uri_to_path(uri);
    om.close_document(&file);
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let mut out = Cursor::new(Vec::new());
    let ok = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "textDocument/willSaveWaitUntil",
            "params": { "textDocument": { "uri": uri }, "reason": 1 }
        }),
        &mut out,
        &mut lc,
        &mut mr,
        &mut om,
        &cancelled,
        None,
    )
    .unwrap_or(false);

    assert!(ok);
    let resp = extract_response(&out.into_inner());
    assert_eq!(resp["result"], json!([]));
    assert_eq!(om.open_count(), 0);
}

#[test]
fn test_will_save_notification_returns_true_in_dispatch() {
    let uri = "file:///tmp/notif_true.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn t() {}\n");
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let mut out = Cursor::new(Vec::new());
    let res = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/willSave",
            "params": { "textDocument": { "uri": uri }, "reason": 1 }
        }),
        &mut out,
        &mut lc,
        &mut mr,
        &mut om,
        &cancelled,
        None,
    );

    assert!(res.unwrap_or(false));
}

#[test]
fn test_will_save_wait_until_jsonrpc_version_2_0() {
    let uri = "file:///tmp/jsonrpc_ver.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn v() {}\n");
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let mut out = Cursor::new(Vec::new());
    let ok = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "textDocument/willSaveWaitUntil",
            "params": { "textDocument": { "uri": uri }, "reason": 1 }
        }),
        &mut out,
        &mut lc,
        &mut mr,
        &mut om,
        &cancelled,
        None,
    )
    .unwrap_or(false);

    assert!(ok);
    let resp = extract_response(&out.into_inner());
    assert_eq!(resp["jsonrpc"], "2.0");
}

#[test]
fn test_will_save_with_arbitrary_client_extensions() {
    let uri = "file:///tmp/client_ext.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn ext() {}\n");
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let mut out = Cursor::new(Vec::new());
    let ok = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/willSave",
            "params": {
                "textDocument": { "uri": uri },
                "reason": 1,
                "clientExtraInfo": { "ide": "vscode", "session": 123 }
            }
        }),
        &mut out,
        &mut lc,
        &mut mr,
        &mut om,
        &cancelled,
        None,
    )
    .unwrap_or(false);

    assert!(ok);
    assert_eq!(out.into_inner().len(), 0);
}

#[test]
fn test_will_save_wait_until_with_arbitrary_client_extensions() {
    let uri = "file:///tmp/client_ext_wait.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn ext() {}\n");
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let mut out = Cursor::new(Vec::new());
    let ok = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "textDocument/willSaveWaitUntil",
            "params": {
                "textDocument": { "uri": uri },
                "reason": 1,
                "clientExtraInfo": { "ide": "neovim" }
            }
        }),
        &mut out,
        &mut lc,
        &mut mr,
        &mut om,
        &cancelled,
        None,
    )
    .unwrap_or(false);

    assert!(ok);
    let resp = extract_response(&out.into_inner());
    assert_eq!(resp["result"], json!([]));
}
