/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 3: `textDocument/didOpen` synchronization tests (part 5: language IDs, URIs, positions, and scopes).

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::{uri_to_path, PositionConverter};
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use ridx_format::DefKind;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::io::Cursor;
use std::sync::{Arc, Mutex};

fn dispatch_did_open_with_converter(
    msg: Value,
    lifecycle: &mut LifecycleManager,
    multi_root: &mut MultiRootWorkspace,
    overlay_mgr: &mut OverlayManager,
    _converter: Option<&mut PositionConverter>,
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
fn test_did_open_with_language_id_rust_handled_safely() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/lang_rust.rs";
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_open_with_converter(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": uri,
                    "languageId": "rust",
                    "version": 1,
                    "text": "fn lang() {}\n"
                }
            }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        None,
    );

    assert_eq!(ok, true);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.version, 1);
    assert_eq!(overlay.text, "fn lang() {}\n");
}

#[test]
fn test_did_open_with_unknown_language_id_safely() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/unknown_lang.rs";
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_open_with_converter(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": uri,
                    "languageId": "toml",
                    "version": 5,
                    "text": "key = 123\n"
                }
            }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        None,
    );

    assert_eq!(ok, true);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.version, 5);
    assert_eq!(overlay.text, "key = 123\n");
}

#[test]
fn test_did_open_symbol_exact_line_and_col() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/exact_pos.rs";
    let file = uri_to_path(uri);
    let code = "pub fn alpha() {}\n";

    let (ok, _) = dispatch_did_open_with_converter(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": code } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        None,
    );

    assert_eq!(ok, true);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.symbols.len(), 1);
    assert_eq!(overlay.symbols[0].name, "alpha");
    assert_eq!(overlay.symbols[0].start_line, 1);
    assert_eq!(overlay.symbols[0].start_col, 8);
    assert_eq!(overlay.symbols[0].end_line, 1);
    assert_eq!(overlay.symbols[0].end_col, 13);
}

#[test]
fn test_did_open_symbol_multiline_offsets() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/multi_offset.rs";
    let file = uri_to_path(uri);
    let code = "\n\npub struct Second;\n";

    let (ok, _) = dispatch_did_open_with_converter(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": code } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        None,
    );

    assert_eq!(ok, true);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.symbols.len(), 1);
    assert_eq!(overlay.symbols[0].name, "Second");
    assert_eq!(overlay.symbols[0].start_line, 3);
    assert_eq!(overlay.symbols[0].kind, DefKind::Struct);
}

#[test]
fn test_did_open_nested_items_symbols_count() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/nested.rs";
    let file = uri_to_path(uri);
    let code = "pub struct Container;\npub fn helper() { let inner = 1; }\n";

    let (ok, _) = dispatch_did_open_with_converter(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": code } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        None,
    );

    assert_eq!(ok, true);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.symbols.len(), 3);
    assert_eq!(overlay.symbols[0].name, "Container");
    assert_eq!(overlay.symbols[1].name, "helper");
    assert_eq!(overlay.symbols[2].name, "inner");
}

#[test]
fn test_did_open_invalid_syntax_rust_preserves_text() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/broken.rs";
    let file = uri_to_path(uri);
    let code = "pub fn broken( { // missing parens\n";

    let (ok, _) = dispatch_did_open_with_converter(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": code } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        None,
    );

    assert_eq!(ok, true);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, code);
}

#[test]
fn test_did_open_escaped_uri_space_handling() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/my%20dir/file.rs";
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_open_with_converter(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": "fn spaced() {}\n" } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        None,
    );

    assert_eq!(ok, true);
    assert_eq!(file, "/tmp/my dir/file.rs");
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "fn spaced() {}\n");
}

#[test]
fn test_did_open_replaces_previously_closed_file() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/reclosed.rs";
    let file = uri_to_path(uri);

    let _ = dispatch_did_open_with_converter(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": { "uri": uri, "version": 1, "text": "fn f1() {}\n" } } }),
        &mut lifecycle, &mut multi_root, &mut overlay_mgr, None,
    );
    overlay_mgr.close_document(&file);
    assert_eq!(overlay_mgr.open_count(), 0);

    let (ok, _) = dispatch_did_open_with_converter(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": { "uri": uri, "version": 2, "text": "fn f2() {}\n" } } }),
        &mut lifecycle, &mut multi_root, &mut overlay_mgr, None,
    );

    assert_eq!(ok, true);
    assert_eq!(overlay_mgr.open_count(), 1);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.version, 2);
    assert_eq!(overlay.text, "fn f2() {}\n");
}

#[test]
fn test_did_open_invalidates_converter_cache() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let temp_dir = std::env::temp_dir().join(format!("conv_open_{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let path = temp_dir.join("conv.rs");
    let _ = fs::write(&path, "fn initial() {}\n");
    let uri = format!("file://{}", path.display());
    let file = uri_to_path(&uri);

    // Warm up converter cache
    let cached = lifecycle.conv.get_line(&file, 1);
    assert_eq!(cached, Some("fn initial() {}"));

    // Write updated content to disk behind cache
    let _ = fs::write(&path, "fn disk_updated() {}\n");
    assert_eq!(lifecycle.conv.get_line(&file, 1), Some("fn initial() {}"));

    let (ok, _) = dispatch_did_open_with_converter(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": "fn evicted() {}\n" } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        None,
    );

    assert_eq!(ok, true);
    assert_eq!(lifecycle.conv.get_line(&file, 1), Some("fn disk_updated() {}"));
    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_did_open_null_text_document_handled_safely() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();

    let (ok, _) = dispatch_did_open_with_converter(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {}
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        None,
    );

    assert_eq!(ok, true);
    assert_eq!(overlay_mgr.open_count(), 1);
}

#[test]
fn test_did_open_content_hash_computed() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/hash_test.rs";
    let file = uri_to_path(uri);
    let code = "pub fn hashed_fn() {}\n";

    let (ok, _) = dispatch_did_open_with_converter(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": code } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        None,
    );

    assert_eq!(ok, true);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.content_hash, ridx_format::normalized_source_hash(code.as_bytes()));
}
