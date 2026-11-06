/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 3: `textDocument/didClose` synchronization tests (part 2: converters, reopening, and dirty states).

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::{uri_to_path, PositionConverter};
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
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

fn dispatch_did_close_with_converter(
    msg: Value,
    lifecycle: &mut LifecycleManager,
    multi_root: &mut MultiRootWorkspace,
    overlay_mgr: &mut OverlayManager,
    _converter: Option<&mut PositionConverter>,
) -> (bool, Vec<u8>) {
    let mut out = Cursor::new(Vec::new());
    let cancelled = Arc::new(Mutex::new(HashSet::new()));
    let res = dispatch_lsp_message(msg, &mut out, lifecycle, multi_root, overlay_mgr, &cancelled, None);
    (res.unwrap_or(false), out.into_inner())
}

#[test]
fn test_did_close_invalidates_position_converter_cache() {
    let temp_dir = std::env::temp_dir().join(format!("conv_close_{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let path = temp_dir.join("conv.rs");
    let _ = fs::write(&path, "line1_initial\n");
    let uri = format!("file://{}", path.display());
    let (mut lc, mut mr, mut om) = setup_server_with_doc(&uri, "line1_initial\n");
    let file = uri_to_path(&uri);

    // Warm up converter cache
    let cached = lc.conv.get_line(&file, 1);
    assert_eq!(cached, Some("line1_initial"));

    // Write updated content to disk behind converter cache
    let _ = fs::write(&path, "line1_disk_updated\n");
    assert_eq!(lc.conv.get_line(&file, 1), Some("line1_initial"));

    let (ok, _) = dispatch_did_close_with_converter(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didClose",
            "params": { "textDocument": { "uri": uri } }
        }),
        &mut lc,
        &mut mr,
        &mut om,
        None,
    );

    assert!(ok);
    assert_eq!(lc.conv.get_line(&file, 1), Some("line1_disk_updated"));
    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_did_close_followed_by_reopen_starts_fresh_version() {
    let uri = "file:///tmp/reopen_fresh.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "v1 text\n");
    let file = uri_to_path(uri);

    // Close
    let (ok_close, _) = dispatch_did_close_with_converter(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri } } }),
        &mut lc,
        &mut mr,
        &mut om,
        None,
    );
    assert!(ok_close);
    assert_eq!(om.open_count(), 0);

    // Reopen with new version 10
    let cancelled = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Cursor::new(Vec::new());
    let ok_open = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 10, "text": "fresh v10\n" } }
        }),
        &mut out,
        &mut lc,
        &mut mr,
        &mut om,
        &cancelled,
        None,
    )
    .unwrap_or(false);

    assert!(ok_open);
    assert_eq!(om.open_count(), 1);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.version, 10);
    assert_eq!(overlay.text, "fresh v10\n");
}

#[test]
fn test_did_close_reopen_with_different_text() {
    let uri = "file:///tmp/diff_text_close.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "old fn()\n");
    let file = uri_to_path(uri);

    let _ = dispatch_did_close_with_converter(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri } } }),
        &mut lc,
        &mut mr,
        &mut om,
        None,
    );

    let cancelled = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Cursor::new(Vec::new());
    let _ = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": "pub struct NewPoint;\n" } }
        }),
        &mut out,
        &mut lc,
        &mut mr,
        &mut om,
        &cancelled,
        None,
    );

    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "pub struct NewPoint;\n");
    assert_eq!(overlay.symbols.len(), 1);
    assert_eq!(overlay.symbols[0].name, "NewPoint");
}

#[test]
fn test_did_close_does_not_affect_unrelated_open_files() {
    let uri1 = "file:///tmp/close_first.rs";
    let uri2 = "file:///tmp/keep_second.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri1, "doc1\n");
    let file2 = uri_to_path(uri2);
    om.open_with_provenance(uri2, &file2, 5, "doc2 intact\n".to_string(), true);

    let (ok, _) = dispatch_did_close_with_converter(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri1 } } }),
        &mut lc,
        &mut mr,
        &mut om,
        None,
    );

    assert!(ok);
    assert_eq!(om.open_count(), 1);
    let overlay2 = om.get(&file2).expect("doc2 must be untouched");
    assert_eq!(overlay2.text, "doc2 intact\n");
    assert_eq!(overlay2.version, 5);
}

#[test]
fn test_did_close_preserves_unrelated_file_version_and_symbols() {
    let uri1 = "file:///tmp/evicted.rs";
    let uri2 = "file:///tmp/preserved.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri1, "fn a() {}\n");
    let file2 = uri_to_path(uri2);
    om.open_with_provenance(uri2, &file2, 3, "pub fn b() {}\n".to_string(), true);

    let (ok, _) = dispatch_did_close_with_converter(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri1 } } }),
        &mut lc,
        &mut mr,
        &mut om,
        None,
    );

    assert!(ok);
    let overlay2 = om.get(&file2).expect("doc2 must be untouched");
    assert_eq!(overlay2.symbols.len(), 1);
    assert_eq!(overlay2.symbols[0].name, "b");
}

#[test]
fn test_did_close_empty_uri_safe() {
    let mut lc = LifecycleManager::new();
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();

    let (ok, out) = dispatch_did_close_with_converter(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didClose",
            "params": { "textDocument": { "uri": "" } }
        }),
        &mut lc,
        &mut mr,
        &mut om,
        None,
    );

    assert!(ok);
    assert_eq!(out.len(), 0);
}

#[test]
fn test_did_close_windows_file_uri_eviction() {
    let uri = "file:///C:/projects/test/workspace/lib.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn win() {}\n");
    let file = uri_to_path(uri);

    assert_eq!(om.open_count(), 1);
    let (ok, _) = dispatch_did_close_with_converter(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri } } }),
        &mut lc,
        &mut mr,
        &mut om,
        None,
    );

    assert!(ok);
    assert_eq!(om.open_count(), 0);
    assert_eq!(om.get(&file).map(|d| d.text.as_str()), None);
}

#[test]
fn test_did_close_url_encoded_space_uri_eviction() {
    let uri = "file:///tmp/my%20space%20dir/file.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn space() {}\n");
    let file = uri_to_path(uri);

    assert_eq!(om.open_count(), 1);
    let (ok, _) = dispatch_did_close_with_converter(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri } } }),
        &mut lc,
        &mut mr,
        &mut om,
        None,
    );

    assert!(ok);
    assert_eq!(om.open_count(), 0);
    assert_eq!(om.get(&file).map(|d| d.text.as_str()), None);
}

#[test]
fn test_did_close_does_not_modify_disk_file() {
    let temp_file = std::env::temp_dir().join("disk_close_immutability.rs");
    let _ = fs::write(&temp_file, "disk original\n");
    let uri = format!("file://{}", temp_file.display());

    let (mut lc, mut mr, mut om) = setup_server_with_doc(&uri, "overlay dirty\n");
    let (ok, _) = dispatch_did_close_with_converter(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri } } }),
        &mut lc,
        &mut mr,
        &mut om,
        None,
    );

    assert!(ok);
    let disk_content = fs::read_to_string(&temp_file).unwrap();
    let _ = fs::remove_file(&temp_file);
    assert_eq!(disk_content, "disk original\n");
}

#[test]
fn test_did_close_discards_unsaved_dirty_overlay() {
    let uri = "file:///tmp/dirty_discard.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "baseline\n");
    let file = uri_to_path(uri);

    om.apply_content_changes(&file, 2, &[json!({ "text": "unsaved dirty text\n" })]);
    assert_eq!(om.get(&file).unwrap().text, "unsaved dirty text\n");

    let (ok, _) = dispatch_did_close_with_converter(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri } } }),
        &mut lc,
        &mut mr,
        &mut om,
        None,
    );

    assert!(ok);
    assert_eq!(om.get(&file).map(|d| d.text.as_str()), None);
}
