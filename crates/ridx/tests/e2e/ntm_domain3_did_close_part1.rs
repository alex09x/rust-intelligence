/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 3: `textDocument/didClose` synchronization tests (part 1: overlay eviction and basic invariants).

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
fn test_did_close_removes_overlay_from_manager() {
    let uri = "file:///tmp/close_evict.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");
    let file = uri_to_path(uri);

    assert_eq!(om.open_count(), 1);
    let (ok, out) = dispatch_did_close(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didClose",
            "params": { "textDocument": { "uri": uri } }
        }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(out.len(), 0);
    assert_eq!(om.open_count(), 0);
    assert_eq!(om.get(&file).map(|d| d.text.as_str()), None);
}

#[test]
fn test_did_close_reduces_open_count_by_one() {
    let mut lc = LifecycleManager::new();
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();

    for i in 1..=3 {
        let uri = format!("file:///tmp/multi_close_{}.rs", i);
        let file = uri_to_path(&uri);
        om.open_with_provenance(&uri, &file, 1, format!("fn f{}() {{}}\n", i), true);
    }
    assert_eq!(om.open_count(), 3);

    let (ok, _) = dispatch_did_close(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didClose",
            "params": { "textDocument": { "uri": "file:///tmp/multi_close_2.rs" } }
        }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(om.open_count(), 2);
}

#[test]
fn test_did_close_open_file_paths_excludes_closed_file() {
    let mut lc = LifecycleManager::new();
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();

    let uri1 = "file:///tmp/keep_me.rs";
    let uri2 = "file:///tmp/evict_me.rs";
    let file1 = uri_to_path(uri1);
    let file2 = uri_to_path(uri2);

    om.open_with_provenance(uri1, &file1, 1, "fn keep() {}\n".to_string(), true);
    om.open_with_provenance(uri2, &file2, 1, "fn evict() {}\n".to_string(), true);

    let (ok, _) = dispatch_did_close(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didClose",
            "params": { "textDocument": { "uri": uri2 } }
        }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(om.open_file_paths(), vec![file1]);
}

#[test]
fn test_did_close_returns_ok_true() {
    let uri = "file:///tmp/ret_ok_close.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");

    let (ok, _) = dispatch_did_close(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didClose",
            "params": { "textDocument": { "uri": uri } }
        }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
}

#[test]
fn test_did_close_produces_no_stdout_output() {
    let uri = "file:///tmp/quiet_close.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");

    let (ok, out) = dispatch_did_close(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didClose",
            "params": { "textDocument": { "uri": uri } }
        }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(out.len(), 0);
}

#[test]
fn test_did_close_unopened_document_returns_ok_true() {
    let mut lc = LifecycleManager::new();
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();

    let (ok, out) = dispatch_did_close(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didClose",
            "params": { "textDocument": { "uri": "file:///tmp/never_opened.rs" } }
        }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(out.len(), 0);
    assert_eq!(om.open_count(), 0);
}

#[test]
fn test_did_close_already_closed_document_is_idempotent() {
    let uri = "file:///tmp/double_close.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");

    let (ok1, _) = dispatch_did_close(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri } } }),
        &mut lc, &mut mr, &mut om,
    );
    assert_eq!(ok1, true);

    let (ok2, _) = dispatch_did_close(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri } } }),
        &mut lc, &mut mr, &mut om,
    );
    assert_eq!(ok2, true);
    assert_eq!(om.open_count(), 0);
}

#[test]
fn test_did_close_empty_params_safe() {
    let mut lc = LifecycleManager::new();
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();

    let (ok, out) = dispatch_did_close(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didClose",
            "params": {}
        }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(out.len(), 0);
}

#[test]
fn test_did_close_missing_uri_safe() {
    let mut lc = LifecycleManager::new();
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();

    let (ok, out) = dispatch_did_close(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didClose",
            "params": { "textDocument": {} }
        }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(out.len(), 0);
}

#[test]
fn test_did_close_null_params_safe() {
    let mut lc = LifecycleManager::new();
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();

    let (ok, out) = dispatch_did_close(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didClose",
            "params": null
        }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(out.len(), 0);
}
