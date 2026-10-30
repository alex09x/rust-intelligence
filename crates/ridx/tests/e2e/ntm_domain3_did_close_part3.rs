/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 3: `textDocument/didClose` synchronization tests (part 3: lifecycle sequences and multi-root).

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

fn dispatch_did_close(msg: Value, lifecycle: &mut LifecycleManager, multi_root: &mut MultiRootWorkspace, overlay_mgr: &mut OverlayManager) -> (bool, Vec<u8>) {
    let mut out = Cursor::new(Vec::new());
    let cancelled = Arc::new(Mutex::new(HashSet::new()));
    let res = dispatch_lsp_message(msg, &mut out, lifecycle, multi_root, overlay_mgr, &cancelled, None);
    (res.unwrap_or(false), out.into_inner())
}

#[test]
fn test_did_close_full_lifecycle_open_change_save_close() {
    let uri = "file:///tmp/full_life.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "initial\n");
    let file = uri_to_path(uri);
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    // didChange
    let mut out1 = Cursor::new(Vec::new());
    let _ = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didChange",
            "params": { "textDocument": { "uri": uri, "version": 2 }, "contentChanges": [{ "text": "edited\n" }] }
        }),
        &mut out1,
        &mut lc,
        &mut mr,
        &mut om,
        &cancelled,
        None,
    );

    // didSave
    let mut out2 = Cursor::new(Vec::new());
    let _ = dispatch_lsp_message(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didSave", "params": { "textDocument": { "uri": uri } } }),
        &mut out2,
        &mut lc,
        &mut mr,
        &mut om,
        &cancelled,
        None,
    );

    // didClose
    let (ok, _) = dispatch_did_close(json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri } } }), &mut lc, &mut mr, &mut om);

    assert_eq!(ok, true);
    assert_eq!(om.open_count(), 0);
    assert_eq!(om.get(&file).map(|d| d.text.as_str()), None);
}

#[test]
fn test_did_close_open_change_close_without_save() {
    let uri = "file:///tmp/close_unsaved.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "initial\n");
    let file = uri_to_path(uri);
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let mut out = Cursor::new(Vec::new());
    let _ = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didChange",
            "params": { "textDocument": { "uri": uri, "version": 2 }, "contentChanges": [{ "text": "never saved\n" }] }
        }),
        &mut out,
        &mut lc,
        &mut mr,
        &mut om,
        &cancelled,
        None,
    );

    let (ok, _) = dispatch_did_close(json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri } } }), &mut lc, &mut mr, &mut om);

    assert_eq!(ok, true);
    assert_eq!(om.open_count(), 0);
    assert_eq!(om.get(&file).map(|d| d.text.as_str()), None);
}

#[test]
fn test_did_close_sequential_closing_all_files() {
    let mut lc = LifecycleManager::new();
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();

    for i in 1..=5 {
        let uri = format!("file:///tmp/close_all_{}.rs", i);
        let file = uri_to_path(&uri);
        om.open_with_provenance(&uri, &file, 1, format!("fn f{}() {{}}\n", i), true);
    }
    assert_eq!(om.open_count(), 5);

    for i in 1..=5 {
        let uri = format!("file:///tmp/close_all_{}.rs", i);
        let (ok, _) = dispatch_did_close(json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri } } }), &mut lc, &mut mr, &mut om);
        assert_eq!(ok, true);
    }

    assert_eq!(om.open_count(), 0);
}

#[test]
fn test_did_close_reverse_order_closing() {
    let mut lc = LifecycleManager::new();
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();

    for i in 1..=3 {
        let uri = format!("file:///tmp/rev_close_{}.rs", i);
        let file = uri_to_path(&uri);
        om.open_with_provenance(&uri, &file, 1, format!("fn f{}() {{}}\n", i), true);
    }

    for i in (1..=3).rev() {
        let uri = format!("file:///tmp/rev_close_{}.rs", i);
        let (ok, _) = dispatch_did_close(json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri } } }), &mut lc, &mut mr, &mut om);
        assert_eq!(ok, true);
    }

    assert_eq!(om.open_count(), 0);
}

#[test]
fn test_did_close_does_not_affect_multi_root_workspaces() {
    let (mut lc, mut mr, mut om) = setup_server_with_doc("file:///tmp/f.rs", "fn f() {}\n");
    let ws_count_before = mr.all_workspaces().count();

    let (ok, _) =
        dispatch_did_close(json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": "file:///tmp/f.rs" } } }), &mut lc, &mut mr, &mut om);

    assert_eq!(ok, true);
    assert_eq!(mr.all_workspaces().count(), ws_count_before);
}

#[test]
fn test_did_close_does_not_alter_cancelled_requests() {
    let uri = "file:///tmp/close_canc.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");
    let cancelled = Arc::new(Mutex::new(HashSet::new()));
    cancelled.lock().unwrap().insert(json!(54321));

    let mut out = Cursor::new(Vec::new());
    let ok = dispatch_lsp_message(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri } } }),
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

#[test]
fn test_did_close_notification_with_extra_client_data() {
    let uri = "file:///tmp/close_extra.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");

    let (ok, out) = dispatch_did_close(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didClose",
            "params": {
                "textDocument": { "uri": uri },
                "clientSessionId": 999
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(out.len(), 0);
    assert_eq!(om.open_count(), 0);
}

#[test]
fn test_did_close_reused_notification_safe() {
    let uri = "file:///tmp/close_reused.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");

    let msg = json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didClose",
        "params": { "textDocument": { "uri": uri } }
    });

    let (ok1, _) = dispatch_did_close(msg.clone(), &mut lc, &mut mr, &mut om);
    let (ok2, _) = dispatch_did_close(msg, &mut lc, &mut mr, &mut om);

    assert_eq!(ok1, true);
    assert_eq!(ok2, true);
    assert_eq!(om.open_count(), 0);
}

#[test]
fn test_did_close_non_file_uri_scheme_safe() {
    let mut lc = LifecycleManager::new();
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();

    let (ok, out) = dispatch_did_close(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didClose",
            "params": { "textDocument": { "uri": "inmemory://buffer-1" } }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(out.len(), 0);
}

#[test]
fn test_did_close_relative_uri_fallback_safe() {
    let uri = "file://rel_doc.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");

    let (ok, _) = dispatch_did_close(json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri } } }), &mut lc, &mut mr, &mut om);

    assert_eq!(ok, true);
    assert_eq!(om.open_count(), 0);
}
