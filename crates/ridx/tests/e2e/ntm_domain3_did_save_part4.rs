/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 3: `textDocument/didSave` synchronization tests (part 4: edit interleaving and reconciliation).

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::uri_to_path;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::Cursor;
use std::sync::{Arc, Mutex};

fn setup_server_with_doc(uri: &str, text: &str) -> (LifecycleManager, MultiRootWorkspace, OverlayManager) {
    let lc = LifecycleManager::new();
    let mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();
    let file = uri_to_path(uri);
    om.open_with_provenance(uri, &file, 1, text.to_string(), true);
    (lc, mr, om)
}

fn dispatch_did_save(msg: Value, lifecycle: &mut LifecycleManager, multi_root: &mut MultiRootWorkspace, overlay_mgr: &mut OverlayManager) -> (bool, Vec<u8>) {
    let mut out = Cursor::new(Vec::new());
    let cancelled = Arc::new(Mutex::new(HashSet::new()));
    let res = dispatch_lsp_message(msg, &mut out, lifecycle, multi_root, overlay_mgr, &cancelled, None);
    (res.unwrap_or(false), out.into_inner())
}

#[test]
fn test_did_save_interleaved_between_changes() {
    let uri = "file:///tmp/interleaved_save.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "base\n");
    let file = uri_to_path(uri);

    // change v2
    om.apply_content_changes(&file, 2, &[json!({ "text": "change v2\n" })]);
    let (ok1, _) = dispatch_did_save(json!({ "jsonrpc": "2.0", "method": "textDocument/didSave", "params": { "textDocument": { "uri": uri } } }), &mut lc, &mut mr, &mut om);
    assert_eq!(ok1, true);
    assert_eq!(om.get(&file).unwrap().disk_text, Some("change v2\n".to_string()));

    // change v3
    om.apply_content_changes(&file, 3, &[json!({ "text": "change v3\n" })]);
    let (ok2, _) = dispatch_did_save(json!({ "jsonrpc": "2.0", "method": "textDocument/didSave", "params": { "textDocument": { "uri": uri } } }), &mut lc, &mut mr, &mut om);
    assert_eq!(ok2, true);
    assert_eq!(om.get(&file).unwrap().disk_text, Some("change v3\n".to_string()));
}

#[test]
fn test_did_save_full_sync_then_save() {
    let uri = "file:///tmp/full_sync_save.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "initial\n");
    let file = uri_to_path(uri);
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let mut out1 = Cursor::new(Vec::new());
    let ok1 = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{ "text": "full sync replacement\n" }]
            }
        }),
        &mut out1,
        &mut lc,
        &mut mr,
        &mut om,
        &cancelled,
        None,
    )
    .unwrap_or(false);
    assert_eq!(ok1, true);

    let (ok2, _) = dispatch_did_save(json!({ "jsonrpc": "2.0", "method": "textDocument/didSave", "params": { "textDocument": { "uri": uri } } }), &mut lc, &mut mr, &mut om);
    assert_eq!(ok2, true);
    assert_eq!(om.get(&file).unwrap().disk_text, Some("full sync replacement\n".to_string()));
}

#[test]
fn test_did_save_large_file_preserves_full_content() {
    let uri = "file:///tmp/large_save.rs";
    let text = "pub fn item() {}\n".repeat(400);
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, &text);
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_save(json!({ "jsonrpc": "2.0", "method": "textDocument/didSave", "params": { "textDocument": { "uri": uri } } }), &mut lc, &mut mr, &mut om);

    assert_eq!(ok, true);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.disk_text, Some(text.clone()));
    assert_eq!(overlay.text.len(), text.len());
}

#[test]
fn test_did_save_empty_file_preserves_empty_content() {
    let uri = "file:///tmp/empty_save.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_save(json!({ "jsonrpc": "2.0", "method": "textDocument/didSave", "params": { "textDocument": { "uri": uri } } }), &mut lc, &mut mr, &mut om);

    assert_eq!(ok, true);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.disk_text, Some("".to_string()));
}

#[test]
fn test_did_save_does_not_modify_open_count() {
    let uri = "file:///tmp/count_save.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");

    assert_eq!(om.open_count(), 1);
    let (ok, _) = dispatch_did_save(json!({ "jsonrpc": "2.0", "method": "textDocument/didSave", "params": { "textDocument": { "uri": uri } } }), &mut lc, &mut mr, &mut om);

    assert_eq!(ok, true);
    assert_eq!(om.open_count(), 1);
}

#[test]
fn test_did_save_reconcile_with_disk_after_save() {
    let uri = "file:///tmp/reconcile_save.rs";
    let code = "fn re() {}\n";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, code);
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_save(json!({ "jsonrpc": "2.0", "method": "textDocument/didSave", "params": { "textDocument": { "uri": uri } } }), &mut lc, &mut mr, &mut om);
    assert_eq!(ok, true);

    let hash = ridx_format::normalized_source_hash(code.as_bytes());
    om.reconcile_with_disk(&file, hash);
    assert_eq!(om.get(&file).unwrap().content_hash, hash);
}

#[test]
fn test_did_save_client_reused_notification() {
    let uri = "file:///tmp/reused_notif.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");

    let msg = json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didSave",
        "params": { "textDocument": { "uri": uri } }
    });

    let (ok1, _) = dispatch_did_save(msg.clone(), &mut lc, &mut mr, &mut om);
    let (ok2, _) = dispatch_did_save(msg, &mut lc, &mut mr, &mut om);

    assert_eq!(ok1, true);
    assert_eq!(ok2, true);
}

#[test]
fn test_did_save_with_extra_client_metadata() {
    let uri = "file:///tmp/meta_save.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");

    let (ok, out) = dispatch_did_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": {
                "textDocument": { "uri": uri },
                "customClientState": { "saveType": "auto", "triggeredBy": "user" }
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(out.len(), 0);
}

#[test]
fn test_did_save_does_not_affect_cancelled_requests() {
    let uri = "file:///tmp/canc_save.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");
    let cancelled = Arc::new(Mutex::new(HashSet::new()));
    cancelled.lock().unwrap().insert(json!(12345));

    let mut out = Cursor::new(Vec::new());
    let ok = dispatch_lsp_message(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didSave", "params": { "textDocument": { "uri": uri } } }),
        &mut out,
        &mut lc,
        &mut mr,
        &mut om,
        &cancelled,
        None,
    )
    .unwrap_or(false);

    assert_eq!(ok, true);
    assert_eq!(cancelled.lock().unwrap().len(), 1);
}
