/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 3: `textDocument/didSave` synchronization tests (part 3: URIs, paths, and encodings).

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

fn dispatch_did_save(
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
fn test_did_save_routes_to_correct_workspace_root() {
    let uri = "file:///tmp/multi_root_save/sub/file.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");

    let (ok, out) = dispatch_did_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": uri } }
        }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(out.len(), 0);
}

#[test]
fn test_did_save_with_windows_uri() {
    let uri = "file:///C:/workspace/crates/app/src/lib.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn win_save() {}\n");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": uri } }
        }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.disk_text, Some("fn win_save() {}\n".to_string()));
}

#[test]
fn test_did_save_with_url_encoded_space_uri() {
    let uri = "file:///tmp/dir%20with%20space/main.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn space_save() {}\n");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": uri } }
        }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.disk_text, Some("fn space_save() {}\n".to_string()));
}

#[test]
fn test_did_save_with_nested_subdirectory_path() {
    let uri = "file:///tmp/a/b/c/d/deep.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn deep() {}\n");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": uri } }
        }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.disk_text, Some("fn deep() {}\n".to_string()));
}

#[test]
fn test_did_save_empty_uri_handled_safely() {
    let mut lc = LifecycleManager::new();
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();

    let (ok, out) = dispatch_did_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": "" } }
        }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(out.len(), 0);
}

#[test]
fn test_did_save_preserves_compiled_text_when_present() {
    let uri = "file:///tmp/compiled_save.rs";
    let code = "fn compiled_fn() {}\n";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, code);
    let file = uri_to_path(uri);
    om.apply_content_changes(&file, 2, &[json!({ "text": "fn modified_fn() {}\n" })]);

    let (ok, _) = dispatch_did_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": uri } }
        }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.compiled_text, Some(code.to_string()));
    assert_eq!(overlay.disk_text, Some("fn modified_fn() {}\n".to_string()));
}

#[test]
fn test_did_save_preserves_provenance_flag() {
    let uri = "file:///tmp/prov_save.rs";
    let mut lc = LifecycleManager::new();
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();
    let file = uri_to_path(uri);
    om.open_with_provenance(uri, &file, 1, "fn prov() {}\n".to_string(), false);

    let (ok, _) = dispatch_did_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": uri } }
        }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.compiled_text, None);
    assert_eq!(overlay.disk_text, Some("fn prov() {}\n".to_string()));
}

#[test]
fn test_did_save_after_close_document_is_safe_noop() {
    let uri = "file:///tmp/close_then_save.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");
    let file = uri_to_path(uri);
    om.close_document(&file);
    assert_eq!(om.open_count(), 0);

    let (ok, out) = dispatch_did_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": uri } }
        }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    assert_eq!(out.len(), 0);
    assert_eq!(om.open_count(), 0);
}

#[test]
fn test_did_save_crlf_document_preserves_crlf_in_disk_text() {
    let uri = "file:///tmp/crlf_save.rs";
    let crlf_code = "line1\r\nline2\r\nline3\r\n";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, crlf_code);
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": uri } }
        }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.disk_text, Some(crlf_code.to_string()));
}

#[test]
fn test_did_save_unicode_document_preserves_unicode_in_disk_text() {
    let uri = "file:///tmp/unicode_save.rs";
    let unicode_code = "// 🚀 Rust intelligence α β γ\npub fn hello_world() {}\n";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, unicode_code);
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": uri } }
        }),
        &mut lc, &mut mr, &mut om,
    );

    assert_eq!(ok, true);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.disk_text, Some(unicode_code.to_string()));
}
