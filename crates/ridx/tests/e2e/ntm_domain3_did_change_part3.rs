/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 3: `textDocument/didChange` synchronization tests (part 3: multiple changes, ordering, and coordinates).

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
fn test_did_change_multiple_changes_in_single_notification() {
    let uri = "file:///tmp/multi_changes.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "aaa\nbbb\nccc\n");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_change(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [
                    {
                        "range": {
                            "start": { "line": 0, "character": 0 },
                            "end": { "line": 0, "character": 3 }
                        },
                        "text": "AAA"
                    },
                    {
                        "range": {
                            "start": { "line": 2, "character": 0 },
                            "end": { "line": 2, "character": 3 }
                        },
                        "text": "CCC"
                    }
                ]
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert_eq!(ok, true);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "AAA\nbbb\nCCC\n");
}

#[test]
fn test_did_change_sequential_inserts_in_single_line() {
    let uri = "file:///tmp/seq_inserts.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "ab");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_change(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [
                    {
                        "range": {
                            "start": { "line": 0, "character": 1 },
                            "end": { "line": 0, "character": 1 }
                        },
                        "text": "X"
                    },
                    {
                        "range": {
                            "start": { "line": 0, "character": 2 },
                            "end": { "line": 0, "character": 2 }
                        },
                        "text": "Y"
                    }
                ]
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert_eq!(ok, true);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "aXYb");
}

#[test]
fn test_did_change_version_increment_from_client() {
    let uri = "file:///tmp/version_inc.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "fn f() {}\n");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_change(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 42 },
                "contentChanges": [{ "text": "fn f2() {}\n" }]
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert_eq!(ok, true);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.version, 42);
}

#[test]
fn test_did_change_version_zero_increments_internal_version() {
    let uri = "file:///tmp/v0_inc.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "fn f() {}\n");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_change(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 0 },
                "contentChanges": [{ "text": "fn f_new() {}\n" }]
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert_eq!(ok, true);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.version, 2);
}

#[test]
fn test_did_change_negative_version_increments_internal_version() {
    let uri = "file:///tmp/v_neg_inc.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "fn f() {}\n");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_change(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": -1 },
                "contentChanges": [{ "text": "fn f_neg() {}\n" }]
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert_eq!(ok, true);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.version, 2);
}

#[test]
fn test_did_change_content_hash_updates_after_edit() {
    let uri = "file:///tmp/hash_update.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "fn hash_orig() {}\n");
    let file = uri_to_path(uri);
    let initial_hash = om.get(&file).unwrap().content_hash;

    let (ok, _) = dispatch_did_change(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{ "text": "fn hash_modified() {}\n" }]
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert_eq!(ok, true);
    let updated_hash = om.get(&file).unwrap().content_hash;
    assert_eq!(updated_hash, ridx_format::normalized_source_hash(b"fn hash_modified() {}\n"));
    assert_eq!(initial_hash == updated_hash, false);
}

#[test]
fn test_did_change_line_starts_recalculated_after_newline_insert() {
    let uri = "file:///tmp/ls_insert.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "single line");
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
                        "end": { "line": 0, "character": 7 }
                    },
                    "text": "\n"
                }]
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert_eq!(ok, true);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "single\nline");
    assert_eq!(overlay.line_starts, vec![0, 7]);
}

#[test]
fn test_did_change_line_starts_recalculated_after_newline_delete() {
    let uri = "file:///tmp/ls_del.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "line1\nline2\n");
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
                        "end": { "line": 1, "character": 0 }
                    },
                    "text": " "
                }]
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert_eq!(ok, true);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "line1 line2\n");
    assert_eq!(overlay.line_starts, vec![0, 12]);
}

#[test]
fn test_did_change_utf16_surrogate_pair_column_offset() {
    let uri = "file:///tmp/emoji_edit.rs";
    // 🦀 is 4 bytes in UTF-8, 2 code units in UTF-16
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "let emoji = '🦀';\n");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_change(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{
                    "range": {
                        "start": { "line": 0, "character": 13 },
                        "end": { "line": 0, "character": 15 }
                    },
                    "text": "🚀"
                }]
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert_eq!(ok, true);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "let emoji = '🚀';\n");
}

#[test]
fn test_did_change_multibyte_utf8_char_offset() {
    let uri = "file:///tmp/greek_edit.rs";
    // α is 2 bytes in UTF-8, 1 code unit in UTF-16
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "let α = 1;\n");
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
                    "text": "β"
                }]
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert_eq!(ok, true);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "let β = 1;\n");
}
