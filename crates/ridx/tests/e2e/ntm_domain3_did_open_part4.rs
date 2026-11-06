/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 3: `textDocument/didOpen` synchronization tests (part 4: overlay managers, paths, reopening, and line starts).

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::uri_to_path;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::Cursor;
use std::sync::{Arc, Mutex};

fn dispatch_did_open(msg: Value, lifecycle: &mut LifecycleManager, multi_root: &mut MultiRootWorkspace, overlay_mgr: &mut OverlayManager) -> (bool, Vec<u8>) {
    let mut out = Cursor::new(Vec::new());
    let cancelled = Arc::new(Mutex::new(HashSet::new()));
    let res = dispatch_lsp_message(msg, &mut out, lifecycle, multi_root, overlay_mgr, &cancelled, None);
    (res.unwrap_or(false), out.into_inner())
}

#[test]
fn test_did_open_open_file_paths_lists_exact_paths() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();

    let uri1 = "file:///tmp/first_doc.rs";
    let uri2 = "file:///tmp/second_doc.rs";
    let file1 = uri_to_path(uri1);
    let file2 = uri_to_path(uri2);

    let _ = dispatch_did_open(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": { "uri": uri1, "version": 1, "text": "fn f1() {}\n" } } }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );
    let _ = dispatch_did_open(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": { "uri": uri2, "version": 1, "text": "fn f2() {}\n" } } }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    let mut paths = overlay_mgr.open_file_paths();
    paths.sort();
    let mut expected = vec![file1, file2];
    expected.sort();
    assert_eq!(paths, expected);
}

#[test]
fn test_did_open_reopen_updates_version_and_content() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/reopen_test.rs";
    let file = uri_to_path(uri);

    let _ = dispatch_did_open(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": { "uri": uri, "version": 1, "text": "fn old() {}\n" } } }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );
    let _ = dispatch_did_open(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": { "uri": uri, "version": 2, "text": "fn new() {}\n" } } }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.version, 2);
    assert_eq!(overlay.text, "fn new() {}\n");
    assert_eq!(overlay.symbols.len(), 1);
    assert_eq!(overlay.symbols[0].name, "new");
}

#[test]
fn test_did_open_reopen_same_uri_does_not_increase_count() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/same_uri.rs";

    for v in 1..=3 {
        let _ = dispatch_did_open(
            json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": { "uri": uri, "version": v, "text": "fn f() {}\n" } } }),
            &mut lifecycle,
            &mut multi_root,
            &mut overlay_mgr,
        );
    }

    assert_eq!(overlay_mgr.open_count(), 1);
}

#[test]
fn test_did_open_notification_produces_no_stdout_output() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/quiet.rs";

    let (ok, out) = dispatch_did_open(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": { "uri": uri, "version": 1, "text": "fn quiet() {}\n" } } }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    assert!(ok);
    assert_eq!(out.len(), 0);
}

#[test]
fn test_did_open_dispatch_returns_ok_true() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/return_ok.rs";

    let (ok, _) = dispatch_did_open(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": { "uri": uri, "version": 1, "text": "fn ret() {}\n" } } }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    assert!(ok);
}

#[test]
fn test_did_open_single_character_text() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/single_char.rs";
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_open(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": { "uri": uri, "version": 1, "text": "x" } } }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    assert!(ok);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "x");
    assert_eq!(overlay.line_starts, vec![0]);
}

#[test]
fn test_did_open_line_starts_multiple_lines() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/multilines.rs";
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_open(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": { "uri": uri, "version": 1, "text": "a\nb\nc\n" } } }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    assert!(ok);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.line_starts, vec![0, 2, 4, 6]);
}

#[test]
fn test_did_open_line_starts_crlf() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/crlf_lines.rs";
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_open(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": { "uri": uri, "version": 1, "text": "a\r\nb\r\n" } } }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    assert!(ok);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.line_starts, vec![0, 3, 6]);
}

#[test]
fn test_did_open_comment_only_document_symbols_empty() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/comments.rs";
    let file = uri_to_path(uri);
    let code = "// Single line comment\n/* Multi line\n   comment */\n";

    let (ok, _) = dispatch_did_open(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": { "uri": uri, "version": 1, "text": code } } }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    assert!(ok);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.symbols.len(), 0);
}

#[test]
fn test_did_open_empty_uri_handled_safely() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();

    let (ok, _) = dispatch_did_open(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": { "uri": "", "version": 1, "text": "fn empty_uri() {}\n" } } }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    assert!(ok);
    assert_eq!(overlay_mgr.open_count(), 1);
}
