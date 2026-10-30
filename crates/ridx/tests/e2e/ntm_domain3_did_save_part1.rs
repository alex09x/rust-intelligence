/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 3: `textDocument/didSave` synchronization tests (part 1: disk text updates and invariants).

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
fn test_did_save_updates_disk_text_to_current_overlay_text() {
    let uri = "file:///tmp/save_disk_text.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "initial text\n");
    let file = uri_to_path(uri);
    om.apply_content_changes(&file, 2, &[json!({ "text": "dirty saved text\n" })]);

    let (ok, out) = dispatch_did_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": uri } }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(out.len(), 0);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.disk_text, Some("dirty saved text\n".to_string()));
    assert_eq!(overlay.text, "dirty saved text\n");
}

#[test]
fn test_did_save_preserves_version() {
    let uri = "file:///tmp/save_version.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");
    let file = uri_to_path(uri);
    om.apply_content_changes(&file, 7, &[json!({ "text": "fn f_v7() {}\n" })]);

    let (ok, _) = dispatch_did_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": uri } }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert_eq!(ok, true);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.version, 7);
}

#[test]
fn test_did_save_preserves_content_hash() {
    let uri = "file:///tmp/save_hash.rs";
    let code = "pub fn hashed() {}\n";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, code);
    let file = uri_to_path(uri);
    let hash_before = om.get(&file).unwrap().content_hash;

    let (ok, _) = dispatch_did_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": uri } }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert_eq!(ok, true);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.content_hash, hash_before);
}

#[test]
fn test_did_save_preserves_symbols() {
    let uri = "file:///tmp/save_symbols.rs";
    let code = "pub struct SavedPoint { x: i32 }\n";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, code);
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": uri } }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert_eq!(ok, true);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.symbols.len(), 1);
    assert_eq!(overlay.symbols[0].name, "SavedPoint");
}

#[test]
fn test_did_save_produces_no_stdout_output() {
    let uri = "file:///tmp/save_quiet.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn quiet() {}\n");

    let (ok, out) = dispatch_did_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": uri } }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(out.len(), 0);
}

#[test]
fn test_did_save_returns_ok_true() {
    let uri = "file:///tmp/save_ok_true.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");

    let (ok, _) = dispatch_did_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": uri } }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert_eq!(ok, true);
}

#[test]
fn test_did_save_unopened_document_is_safe_noop() {
    let mut lc = LifecycleManager::new();
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();
    let uri = "file:///tmp/unopened_did_save.rs";

    let (ok, out) = dispatch_did_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": uri } }
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
fn test_did_save_empty_params_safe() {
    let mut lc = LifecycleManager::new();
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();

    let (ok, out) = dispatch_did_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": {}
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(out.len(), 0);
}

#[test]
fn test_did_save_missing_uri_safe() {
    let mut lc = LifecycleManager::new();
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();

    let (ok, out) = dispatch_did_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": { "textDocument": {} }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(out.len(), 0);
}

#[test]
fn test_did_save_null_params_safe() {
    let mut lc = LifecycleManager::new();
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();

    let (ok, out) = dispatch_did_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": null
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(out.len(), 0);
}
