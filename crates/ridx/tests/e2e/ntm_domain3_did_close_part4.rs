/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 3: `textDocument/didClose` synchronization tests (part 4: fallback and post-close behavior).

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::uri_to_path;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
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

fn dispatch_did_close(
    msg: Value,
    lifecycle: &mut LifecycleManager,
    multi_root: &mut MultiRootWorkspace,
    overlay_mgr: &mut OverlayManager,
) -> (bool, Vec<u8>) {
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
    (res.unwrap_or(false), out.into_inner())
}

#[test]
fn test_did_close_subsequent_did_change_triggers_disk_fallback() {
    let temp_file = std::env::temp_dir().join("close_fallback.rs");
    let _ = fs::write(&temp_file, "disk content\n");
    let uri = format!("file://{}", temp_file.display());
    let file = uri_to_path(&uri);

    let (mut lc, mut mr, mut om) = setup_server_with_doc(&uri, "initial overlay\n");
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    // Close
    let (ok_close, _) = dispatch_did_close(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri } } }),
        &mut lc, &mut mr, &mut om,
    );
    assert_eq!(ok_close, true);
    assert_eq!(om.open_count(), 0);

    // didChange triggers read from disk fallback
    let mut out = Cursor::new(Vec::new());
    let ok_change = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{ "text": "freshly changed\n" }]
            }
        }),
        &mut out, &mut lc, &mut mr, &mut om, &cancelled, None,
    ).unwrap_or(false);

    let _ = fs::remove_file(&temp_file);
    assert_eq!(ok_change, true);
    assert_eq!(om.open_count(), 1);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "freshly changed\n");
}

#[test]
fn test_did_close_subsequent_did_save_is_noop() {
    let uri = "file:///tmp/close_then_save_noop.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let (ok1, _) = dispatch_did_close(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri } } }),
        &mut lc, &mut mr, &mut om,
    );
    assert_eq!(ok1, true);

    let mut out = Cursor::new(Vec::new());
    let ok2 = dispatch_lsp_message(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didSave", "params": { "textDocument": { "uri": uri } } }),
        &mut out, &mut lc, &mut mr, &mut om, &cancelled, None,
    ).unwrap_or(false);

    assert_eq!(ok2, true);
    assert_eq!(om.open_count(), 0);
}

#[test]
fn test_did_close_subsequent_will_save_is_noop() {
    let uri = "file:///tmp/close_will_save_noop.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let (ok1, _) = dispatch_did_close(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri } } }),
        &mut lc, &mut mr, &mut om,
    );
    assert_eq!(ok1, true);

    let mut out = Cursor::new(Vec::new());
    let ok2 = dispatch_lsp_message(
        json!({ "jsonrpc": "2.0", "method": "textDocument/willSave", "params": { "textDocument": { "uri": uri }, "reason": 1 } }),
        &mut out, &mut lc, &mut mr, &mut om, &cancelled, None,
    ).unwrap_or(false);

    assert_eq!(ok2, true);
    assert_eq!(om.open_count(), 0);
}

#[test]
fn test_did_close_subsequent_will_save_wait_until_returns_empty() {
    let uri = "file:///tmp/close_wait_until_empty.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let (ok1, _) = dispatch_did_close(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri } } }),
        &mut lc, &mut mr, &mut om,
    );
    assert_eq!(ok1, true);

    let mut out = Cursor::new(Vec::new());
    let ok2 = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "id": 99,
            "method": "textDocument/willSaveWaitUntil",
            "params": { "textDocument": { "uri": uri }, "reason": 1 }
        }),
        &mut out, &mut lc, &mut mr, &mut om, &cancelled, None,
    ).unwrap_or(false);

    assert_eq!(ok2, true);
    let resp = extract_response(&out.into_inner());
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_did_close_large_document_overlay_freed() {
    let uri = "file:///tmp/large_close.rs";
    let text = "pub fn item() {}\n".repeat(500);
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, &text);
    let file = uri_to_path(uri);

    assert_eq!(om.open_count(), 1);
    let (ok, _) = dispatch_did_close(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri } } }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(om.open_count(), 0);
    assert_eq!(om.get(&file).map(|d| d.text.as_str()), None);
}

#[test]
fn test_did_close_crlf_document_evicted_cleanly() {
    let uri = "file:///tmp/crlf_close.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "line1\r\nline2\r\n");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_close(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri } } }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(om.get(&file).map(|d| d.text.as_str()), None);
}

#[test]
fn test_did_close_unicode_document_evicted_cleanly() {
    let uri = "file:///tmp/unicode_close.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "// 🚀🦀 α\n");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_close(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri } } }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(om.get(&file).map(|d| d.text.as_str()), None);
}

#[test]
fn test_did_close_preserves_flycheck_state() {
    let uri = "file:///tmp/fc_close.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");

    assert_eq!(lc.flycheck.current_generation(), 0);
    let (ok, _) = dispatch_did_close(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri } } }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(lc.flycheck.current_generation(), 0);
}

#[test]
fn test_did_close_single_character_uri_safe() {
    let mut lc = LifecycleManager::new();
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();

    let (ok, out) = dispatch_did_close(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": "file:///x" } } }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(out.len(), 0);
}
