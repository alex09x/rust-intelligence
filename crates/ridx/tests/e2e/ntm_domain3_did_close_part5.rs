/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 3: `textDocument/didClose` synchronization tests (part 5: stress, pairs, and URI variants).

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
fn test_did_close_stress_twenty_open_and_close_cycles() {
    let mut lc = LifecycleManager::new();
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();
    let cancelled = Arc::new(Mutex::new(HashSet::new()));
    let uri = "file:///tmp/cycle.rs";

    for cycle in 1..=20 {
        let mut out1 = Cursor::new(Vec::new());
        let ok_open = dispatch_lsp_message(
            json!({
                "jsonrpc": "2.0", "method": "textDocument/didOpen",
                "params": { "textDocument": { "uri": uri, "version": cycle, "text": format!("fn cycle{}() {{}}\n", cycle) } }
            }),
            &mut out1,
            &mut lc,
            &mut mr,
            &mut om,
            &cancelled,
            None,
        )
        .unwrap_or(false);
        assert!(ok_open);
        assert_eq!(om.open_count(), 1);

        let (ok_close, _) = dispatch_did_close_with_converter(
            json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri } } }),
            &mut lc,
            &mut mr,
            &mut om,
            None,
        );
        assert!(ok_close);
        assert_eq!(om.open_count(), 0);
    }
}

#[test]
fn test_did_close_stress_ten_distinct_files_open_close() {
    let mut lc = LifecycleManager::new();
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();

    for i in 1..=10 {
        let uri = format!("file:///tmp/ten_{}.rs", i);
        let file = uri_to_path(&uri);
        om.open_with_provenance(&uri, &file, 1, "fn f() {}\n".to_string(), true);
    }
    assert_eq!(om.open_count(), 10);

    for i in 1..=10 {
        let uri = format!("file:///tmp/ten_{}.rs", i);
        let (ok, _) = dispatch_did_close_with_converter(
            json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri } } }),
            &mut lc,
            &mut mr,
            &mut om,
            None,
        );
        assert!(ok);
    }
    assert_eq!(om.open_count(), 0);
}

#[test]
fn test_did_close_interleaved_open_close_pairs() {
    let mut lc = LifecycleManager::new();
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let uri_a = "file:///tmp/pair_a.rs";
    let uri_b = "file:///tmp/pair_b.rs";

    // Open A and B
    let mut out1 = Cursor::new(Vec::new());
    let _ = dispatch_lsp_message(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": { "uri": uri_a, "version": 1, "text": "fn a() {}\n" } } }),
        &mut out1,
        &mut lc,
        &mut mr,
        &mut om,
        &cancelled,
        None,
    );
    let mut out2 = Cursor::new(Vec::new());
    let _ = dispatch_lsp_message(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": { "uri": uri_b, "version": 1, "text": "fn b() {}\n" } } }),
        &mut out2,
        &mut lc,
        &mut mr,
        &mut om,
        &cancelled,
        None,
    );
    assert_eq!(om.open_count(), 2);

    // Close A
    let (ok_a, _) = dispatch_did_close_with_converter(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri_a } } }),
        &mut lc,
        &mut mr,
        &mut om,
        None,
    );
    assert!(ok_a);
    assert_eq!(om.open_count(), 1);

    // Close B
    let (ok_b, _) = dispatch_did_close_with_converter(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri_b } } }),
        &mut lc,
        &mut mr,
        &mut om,
        None,
    );
    assert!(ok_b);
    assert_eq!(om.open_count(), 0);
}

#[test]
fn test_did_close_does_not_clear_other_converter_entries() {
    let temp_dir = std::env::temp_dir().join(format!("conv_entries_{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let path1 = temp_dir.join("f1.rs");
    let path2 = temp_dir.join("f2.rs");
    let _ = fs::write(&path1, "line1_f1\n");
    let _ = fs::write(&path2, "line1_f2\n");
    let uri1 = format!("file://{}", path1.display());
    let uri2 = format!("file://{}", path2.display());

    let (mut lc, mut mr, mut om) = setup_server_with_doc(&uri1, "line1_f1\n");
    let file1 = uri_to_path(&uri1);
    let file2 = uri_to_path(&uri2);
    om.open_with_provenance(&uri2, &file2, 1, "line1_f2\n".to_string(), true);

    // Warm up converter cache for both files
    assert_eq!(lc.conv.get_line(&file1, 1), Some("line1_f1"));
    assert_eq!(lc.conv.get_line(&file2, 1), Some("line1_f2"));

    // Write updated content to disk for both files
    let _ = fs::write(&path1, "line1_f1_disk_updated\n");
    let _ = fs::write(&path2, "line1_f2_disk_updated\n");

    // Close only file 1
    let (ok, _) = dispatch_did_close_with_converter(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri1 } } }),
        &mut lc,
        &mut mr,
        &mut om,
        None,
    );

    assert!(ok);
    // File 1 cache was invalidated and now sees updated disk content
    assert_eq!(lc.conv.get_line(&file1, 1), Some("line1_f1_disk_updated"));
    // File 2 cache was NOT cleared and still sees old cached line
    assert_eq!(lc.conv.get_line(&file2, 1), Some("line1_f2"));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_did_close_deep_nested_path() {
    let uri = "file:///tmp/x/y/z/w/v/deep_close.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn deep() {}\n");
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
fn test_did_close_trailing_slash_uri_safe() {
    let uri = "file:///tmp/dir_slash/";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "");

    let (ok, out) = dispatch_did_close_with_converter(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri } } }),
        &mut lc,
        &mut mr,
        &mut om,
        None,
    );

    assert!(ok);
    assert_eq!(out.len(), 0);
    assert_eq!(om.open_count(), 0);
}

#[test]
fn test_did_close_query_string_uri_safe() {
    let uri = "file:///tmp/query.rs?query=true";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn query() {}\n");

    let (ok, _) = dispatch_did_close_with_converter(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri } } }),
        &mut lc,
        &mut mr,
        &mut om,
        None,
    );

    assert!(ok);
    assert_eq!(om.open_count(), 0);
}

#[test]
fn test_did_close_fragment_uri_safe() {
    let uri = "file:///tmp/fragment.rs#L10";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn frag() {}\n");

    let (ok, _) = dispatch_did_close_with_converter(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri } } }),
        &mut lc,
        &mut mr,
        &mut om,
        None,
    );

    assert!(ok);
    assert_eq!(om.open_count(), 0);
}
