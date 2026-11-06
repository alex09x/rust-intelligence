/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 3: `textDocument/didChange` synchronization tests (part 2: full sync and range splices).

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::uri_to_path;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
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
fn test_did_change_full_sync_replaces_entire_buffer() {
    let uri = "file:///tmp/full_sync.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "fn old() {}\n");
    let file = uri_to_path(uri);

    let (ok, out) = dispatch_did_change(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{ "text": "fn replacement() {}\n" }]
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert!(ok);
    assert_eq!(out.len(), 0);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.version, 2);
    assert_eq!(overlay.text, "fn replacement() {}\n");
}

#[test]
fn test_did_change_insert_at_start_of_document() {
    let uri = "file:///tmp/insert_start.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "world\n");
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
                    "text": "hello "
                }]
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert!(ok);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "hello world\n");
}

#[test]
fn test_did_change_insert_at_end_of_document() {
    let uri = "file:///tmp/insert_end.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "hello");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_change(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{
                    "range": {
                        "start": { "line": 0, "character": 5 },
                        "end": { "line": 0, "character": 5 }
                    },
                    "text": " world"
                }]
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert!(ok);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "hello world");
}

#[test]
fn test_did_change_insert_in_middle_of_line() {
    let uri = "file:///tmp/insert_mid.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "fn abc() {}\n");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_change(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{
                    "range": {
                        "start": { "line": 0, "character": 6 },
                        "end": { "line": 0, "character": 6 }
                    },
                    "text": "_123"
                }]
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert!(ok);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "fn abc_123() {}\n");
}

#[test]
fn test_did_change_delete_entire_document() {
    let uri = "file:///tmp/delete_all.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "fn delete_me() {}\n");
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
                        "end": { "line": 1, "character": 0 }
                    },
                    "text": ""
                }]
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert!(ok);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "");
}

#[test]
fn test_did_change_delete_first_character() {
    let uri = "file:///tmp/del_first.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "12345");
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
                        "end": { "line": 0, "character": 1 }
                    },
                    "text": ""
                }]
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert!(ok);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "2345");
}

#[test]
fn test_did_change_delete_last_character() {
    let uri = "file:///tmp/del_last.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "12345");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_change(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{
                    "range": {
                        "start": { "line": 0, "character": 4 },
                        "end": { "line": 0, "character": 5 }
                    },
                    "text": ""
                }]
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert!(ok);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "1234");
}

#[test]
fn test_did_change_replace_word_in_line() {
    let uri = "file:///tmp/replace_word.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "let foo = 1;\n");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_change(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{
                    "range": {
                        "start": { "line": 0, "character": 4 },
                        "end": { "line": 0, "character": 7 }
                    },
                    "text": "bar_baz"
                }]
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert!(ok);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "let bar_baz = 1;\n");
}

#[test]
fn test_did_change_replace_multiline_block() {
    let uri = "file:///tmp/replace_block.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "fn outer() {\n    let a = 1;\n    let b = 2;\n}\n");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_change(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{
                    "range": {
                        "start": { "line": 1, "character": 4 },
                        "end": { "line": 3, "character": 0 }
                    },
                    "text": "let c = 3;\n"
                }]
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert!(ok);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "fn outer() {\n    let c = 3;\n}\n");
}

#[test]
fn test_did_change_empty_change_leaves_text_intact() {
    let uri = "file:///tmp/noop_change.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "fn stable() {}\n");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_change(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{
                    "range": {
                        "start": { "line": 0, "character": 3 },
                        "end": { "line": 0, "character": 3 }
                    },
                    "text": ""
                }]
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert!(ok);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "fn stable() {}\n");
    assert_eq!(overlay.version, 2);
}
