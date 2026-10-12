/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 3: `textDocument/didOpen` synchronization tests (part 3: function modifiers, versions, and limits).

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

fn dispatch_did_open(
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
fn test_did_open_async_fn_symbol() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/async_fn.rs";
    let file = uri_to_path(uri);
    let code = "pub async fn load_data() -> u32 { 100 }\n";

    let (ok, _) = dispatch_did_open(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": code } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    assert_eq!(ok, true);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.symbols.len(), 1);
    assert_eq!(overlay.symbols[0].name, "load_data");
    assert_eq!(overlay.symbols[0].kind, DefKind::Fn);
}

#[test]
fn test_did_open_const_fn_symbol() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/const_fn.rs";
    let file = uri_to_path(uri);
    let code = "pub fn multiplier() -> i32 { 5 }\n";

    let (ok, _) = dispatch_did_open(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": code } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    assert_eq!(ok, true);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.symbols.len(), 1);
    assert_eq!(overlay.symbols[0].name, "multiplier");
    assert_eq!(overlay.symbols[0].kind, DefKind::Fn);
}

#[test]
fn test_did_open_unsafe_fn_symbol() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/unsafe_fn.rs";
    let file = uri_to_path(uri);
    let code = "pub unsafe fn dangerous_operation() {}\n";

    let (ok, _) = dispatch_did_open(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": code } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    assert_eq!(ok, true);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.symbols.len(), 1);
    assert_eq!(overlay.symbols[0].name, "dangerous_operation");
    assert_eq!(overlay.symbols[0].kind, DefKind::Fn);
}

#[test]
fn test_did_open_extern_fn_symbol() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/extern_fn.rs";
    let file = uri_to_path(uri);
    let code = "pub extern \"C\" fn export_c_api() {}\n";

    let (ok, _) = dispatch_did_open(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": code } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    assert_eq!(ok, true);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.symbols.len(), 1);
    assert_eq!(overlay.symbols[0].name, "export_c_api");
    assert_eq!(overlay.symbols[0].kind, DefKind::Fn);
}

#[test]
fn test_did_open_version_zero_preserved() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/v0_test.rs";
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_open(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 0, "text": "fn zero() {}\n" } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    assert_eq!(ok, true);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.version, 0);
}

#[test]
fn test_did_open_negative_version_preserved() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/neg_v_test.rs";
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_open(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": -1, "text": "fn neg() {}\n" } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    assert_eq!(ok, true);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.version, -1);
}

#[test]
fn test_did_open_large_version_preserved() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/large_v_test.rs";
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_open(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": i32::MAX, "text": "fn max_v() {}\n" } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    assert_eq!(ok, true);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.version, i32::MAX);
}

#[test]
fn test_did_open_missing_version_defaults_to_one() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/no_version.rs";
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_open(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "text": "fn default_v() {}\n" } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    assert_eq!(ok, true);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.version, 1);
}

#[test]
fn test_did_open_missing_text_defaults_to_empty() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/no_text.rs";
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_open(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 4 } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    assert_eq!(ok, true);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "");
    assert_eq!(overlay.version, 4);
    assert_eq!(overlay.symbols.len(), 0);
}

#[test]
fn test_did_open_sequential_multiple_files_open_count() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();

    for i in 1..=3 {
        let uri = format!("file:///tmp/seq_file_{}.rs", i);
        let (ok, _) = dispatch_did_open(
            json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": { "textDocument": { "uri": uri, "version": i, "text": format!("fn f{}() {{}}\n", i) } }
            }),
            &mut lifecycle,
            &mut multi_root,
            &mut overlay_mgr,
        );
        assert_eq!(ok, true);
    }

    assert_eq!(overlay_mgr.open_count(), 3);
}
