/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 3: `textDocument/didChange` synchronization tests (part 4: symbols, scopes, and notifications).

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::uri_to_path;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use ridx_format::DefKind;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::Cursor;
use std::sync::{Arc, Mutex};

fn setup_open_doc(uri: &str, initial_text: &str) -> (LifecycleManager, MultiRootWorkspace, OverlayManager) {
    let lifecycle = LifecycleManager::new();
    let multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let file = uri_to_path(uri);
    overlay_mgr.open_with_provenance(uri, &file, 1, initial_text.to_string(), true);
    (lifecycle, multi_root, overlay_mgr)
}

fn dispatch_did_change(msg: Value, lifecycle: &mut LifecycleManager, multi_root: &mut MultiRootWorkspace, overlay_mgr: &mut OverlayManager) -> (bool, Vec<u8>) {
    let mut out = Cursor::new(Vec::new());
    let cancelled = Arc::new(Mutex::new(HashSet::new()));
    let res = dispatch_lsp_message(msg, &mut out, lifecycle, multi_root, overlay_mgr, &cancelled, None);
    (res.unwrap_or(false), out.into_inner())
}

#[test]
fn test_did_change_adds_new_fn_symbol() {
    let uri = "file:///tmp/add_fn.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "// empty\n");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_change(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{ "text": "pub fn added() {}\n" }]
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert!(ok);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.symbols.len(), 1);
    assert_eq!(overlay.symbols[0].name, "added");
    assert_eq!(overlay.symbols[0].kind, DefKind::Fn);
}

#[test]
fn test_did_change_removes_existing_fn_symbol() {
    let uri = "file:///tmp/rem_fn.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "pub fn remove_me() {}\n");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_change(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{ "text": "// replaced with comment\n" }]
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert!(ok);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.symbols.len(), 0);
}

#[test]
fn test_did_change_renames_fn_symbol_in_place() {
    let uri = "file:///tmp/rename_fn.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "pub fn before() {}\n");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_change(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{
                    "range": {
                        "start": { "line": 0, "character": 7 },
                        "end": { "line": 0, "character": 13 }
                    },
                    "text": "after"
                }]
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert!(ok);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.symbols.len(), 1);
    assert_eq!(overlay.symbols[0].name, "after");
}

#[test]
fn test_did_change_adds_struct_definition_symbol() {
    let uri = "file:///tmp/add_struct.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "\n");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_change(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{ "text": "pub struct Config;\n" }]
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert!(ok);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.symbols.len(), 1);
    assert_eq!(overlay.symbols[0].name, "Config");
    assert_eq!(overlay.symbols[0].kind, DefKind::Struct);
}

#[test]
fn test_did_change_adds_enum_definition_symbol() {
    let uri = "file:///tmp/add_enum.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "\n");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_change(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{ "text": "pub enum Action { Play, Stop }\n" }]
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert!(ok);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.symbols.len(), 1);
    assert_eq!(overlay.symbols[0].name, "Action");
    assert_eq!(overlay.symbols[0].kind, DefKind::Enum);
}

#[test]
fn test_did_change_converts_fn_to_comment_removes_symbol() {
    let uri = "file:///tmp/comment_out.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "fn target() {}\n");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_change(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{
                    "range": {
                        "start": { "line": 0, "character": 0 },
                        "end": { "line": 0, "character": 0 }
                    },
                    "text": "// "
                }]
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert!(ok);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "// fn target() {}\n");
    assert_eq!(overlay.symbols.len(), 0);
}

#[test]
fn test_did_change_notification_produces_no_stdout_when_pull_diagnostics() {
    let uri = "file:///tmp/pull_diag.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "fn f() {}\n");

    let (ok, out) = dispatch_did_change(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{ "text": "fn f2() {}\n" }]
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert!(ok);
    // In pull diagnostics mode, no publishDiagnostics is written to out
    assert_eq!(out.len(), 0);
}

#[test]
fn test_did_change_notification_returns_ok_true() {
    let uri = "file:///tmp/ok_true.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "fn f() {}\n");

    let (ok, _) = dispatch_did_change(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{ "text": "fn f3() {}\n" }]
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert!(ok);
}

#[test]
fn test_did_change_empty_content_changes_array_safe() {
    let uri = "file:///tmp/empty_changes.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "fn keep() {}\n");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_change(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 3 },
                "contentChanges": []
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert!(ok);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "fn keep() {}\n");
    assert_eq!(overlay.version, 3);
}

#[test]
fn test_did_change_missing_content_changes_key_safe() {
    let uri = "file:///tmp/no_changes_key.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "fn stay() {}\n");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_change(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 4 }
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert!(ok);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "fn stay() {}\n");
}
