/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 3: Document open synchronization tests (`didOpen`, provenance, CRLF, unicode, and caching).

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::{uri_to_path, PositionConverter};
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::Cursor;
use std::sync::{Arc, Mutex};

fn dispatch_msg(
    msg: Value,
    lifecycle: &mut LifecycleManager,
    multi_root: &mut MultiRootWorkspace,
    overlay_mgr: &mut OverlayManager,
) {
    let mut out = Cursor::new(Vec::new());
    let cancelled = Arc::new(Mutex::new(HashSet::new()));
    let _ = dispatch_lsp_message(
        msg,
        &mut out,
        lifecycle,
        multi_root,
        overlay_mgr,
        &cancelled,
        None,
    );
}

#[test]
fn test_did_open_fresh_document() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();

    let uri = "file:///tmp/fresh_test.rs";
    let file = uri_to_path(uri);
    let code = "pub fn add(a: i32, b: i32) -> i32 { a + b }\n";

    dispatch_msg(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": code } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    assert_eq!(overlay_mgr.open_count(), 1);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.version, 1);
    assert_eq!(overlay.text, code);
    assert!(!overlay.symbols.is_empty());
}

#[test]
fn test_did_open_with_provenance_compiled() {
    let mut overlay_mgr = OverlayManager::new();
    let path = "/test/compiled_doc.rs";
    overlay_mgr.open_with_provenance(
        "file:///test/compiled_doc.rs",
        path,
        1,
        "fn f() {}\n".to_string(),
        true,
    );

    let ov = overlay_mgr.get(path).unwrap();
    assert!(!ov.is_dirty(), "compiled baseline must not be dirty");
    assert_eq!(ov.map_line_to_compiled(1), Some(1));
}

#[test]
fn test_did_open_with_provenance_uncompiled() {
    let mut overlay_mgr = OverlayManager::new();
    let path = "/test/uncompiled_doc.rs";
    overlay_mgr.open_with_provenance(
        "file:///test/uncompiled_doc.rs",
        path,
        1,
        "fn f() {}\n".to_string(),
        false,
    );

    let ov = overlay_mgr.get(path).unwrap();
    assert!(ov.is_dirty(), "uncompiled doc must report dirty");
    assert_eq!(ov.map_line_to_compiled(1), None);
}

#[test]
fn test_did_open_already_open_replaces_cleanly() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();

    let uri = "file:///tmp/replace_test.rs";
    let file = uri_to_path(uri);

    dispatch_msg(
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": "fn v1() {}\n" } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );
    assert_eq!(overlay_mgr.open_count(), 1);

    dispatch_msg(
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 2, "text": "fn v2() {}\n" } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );
    assert_eq!(overlay_mgr.open_count(), 1);
    let ov = overlay_mgr.get(&file).unwrap();
    assert_eq!(ov.version, 2);
    assert!(ov.text.contains("v2"));
}

#[test]
fn test_did_open_crlf_line_endings_handling() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();

    let uri = "file:///tmp/crlf_test.rs";
    let file = uri_to_path(uri);
    let crlf_code = "line1\r\nline2\r\nline3\r\n";

    dispatch_msg(
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": crlf_code } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    let ov = overlay_mgr.get(&file).unwrap();
    assert_eq!(ov.line_starts.len(), 4);
    assert_eq!(ov.position_to_byte_offset(0, 0), 0);
    assert_eq!(ov.position_to_byte_offset(1, 0), 7);
    assert_eq!(ov.position_to_byte_offset(2, 0), 14);
}

#[test]
fn test_did_open_empty_document() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();

    let uri = "file:///tmp/empty_doc.rs";
    let file = uri_to_path(uri);

    dispatch_msg(
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": "" } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    let ov = overlay_mgr.get(&file).unwrap();
    assert_eq!(ov.text, "");
    assert!(ov.symbols.is_empty());
    assert_eq!(ov.position_to_byte_offset(0, 0), 0);
}

#[test]
fn test_did_open_multibyte_unicode_and_emojis() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();

    let uri = "file:///tmp/unicode_doc.rs";
    let file = uri_to_path(uri);
    let code = "fn unicode_greeting() { let 🚀 = 42; }\n";

    dispatch_msg(
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": code } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    let ov = overlay_mgr.get(&file).unwrap();
    let def = ov
        .find_definition_at(1, 4)
        .expect("should find unicode_greeting");
    assert_eq!(def.name, "unicode_greeting");
}

#[test]
fn test_did_open_invalidates_position_cache() {
    let mut conv = PositionConverter::new();
    let file = "/tmp/cache_inval.rs";
    let _ = conv.get_line(file, 1);

    conv.invalidate(file);
    let mut lifecycle = LifecycleManager::new();
    lifecycle.conv.invalidate(file);
    assert_eq!(lifecycle.conv.get_line(file, 1), None);
}
