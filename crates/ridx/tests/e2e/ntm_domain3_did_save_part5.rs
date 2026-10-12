/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 3: `textDocument/didSave` synchronization tests (part 5: batches, schemes, scopes, and flycheck).

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::uri_to_path;
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
fn test_did_save_sequential_batch_across_ten_files() {
    let mut lc = LifecycleManager::new();
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();

    for i in 1..=10 {
        let uri = format!("file:///tmp/batch_save_{}.rs", i);
        let file = uri_to_path(&uri);
        om.open_with_provenance(&uri, &file, 1, format!("fn f{}() {{}}\n", i), true);

        let (ok, _) = dispatch_did_save(
            json!({ "jsonrpc": "2.0", "method": "textDocument/didSave", "params": { "textDocument": { "uri": uri } } }),
            &mut lc, &mut mr, &mut om,
        );
        assert!(ok);
        assert_eq!(om.get(&file).unwrap().disk_text, Some(format!("fn f{}() {{}}\n", i)));
    }
}

#[test]
fn test_did_save_non_file_uri_scheme_safe() {
    let mut lc = LifecycleManager::new();
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();

    let (ok, out) = dispatch_did_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": "untitled:Untitled-1" } }
        }),
        &mut lc, &mut mr, &mut om,
    );

    assert!(ok);
    assert_eq!(out.len(), 0);
}

#[test]
fn test_did_save_relative_path_fallback_safe() {
    let uri = "file://relative_test.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");

    let (ok, _) = dispatch_did_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": uri } }
        }),
        &mut lc, &mut mr, &mut om,
    );

    assert!(ok);
}

#[test]
fn test_did_save_preserves_line_starts_on_nested_scopes() {
    let uri = "file:///tmp/scope_save.rs";
    let code = "fn outer() { let a = 1; { let b = 2; } }\n";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, code);
    let file = uri_to_path(uri);
    let line_starts_before = om.get(&file).unwrap().line_starts.len();

    let (ok, _) = dispatch_did_save(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didSave", "params": { "textDocument": { "uri": uri } } }),
        &mut lc, &mut mr, &mut om,
    );

    assert!(ok);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.line_starts.len(), line_starts_before);
}

#[test]
fn test_did_save_preserves_symbol_locations() {
    let uri = "file:///tmp/sym_loc_save.rs";
    let code = "pub fn located() {}\n";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, code);
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_save(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didSave", "params": { "textDocument": { "uri": uri } } }),
        &mut lc, &mut mr, &mut om,
    );

    assert!(ok);
    let sym = &om.get(&file).unwrap().symbols[0];
    assert_eq!(sym.name, "located");
    assert_eq!(sym.start_line, 1);
    assert_eq!(sym.start_col, 8);
    assert_eq!(sym.end_line, 1);
    assert_eq!(sym.end_col, 15);
}

#[test]
fn test_did_save_preserves_document_open_paths() {
    let uri1 = "file:///tmp/open_p1.rs";
    let uri2 = "file:///tmp/open_p2.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri1, "fn p1() {}\n");
    let file2 = uri_to_path(uri2);
    om.open_with_provenance(uri2, &file2, 1, "fn p2() {}\n".to_string(), true);

    let paths_before = om.open_file_paths();

    let (ok, _) = dispatch_did_save(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didSave", "params": { "textDocument": { "uri": uri1 } } }),
        &mut lc, &mut mr, &mut om,
    );

    assert!(ok);
    assert_eq!(om.open_file_paths(), paths_before);
}

#[test]
fn test_did_save_when_flycheck_already_running() {
    let temp_dir = std::env::temp_dir().join("fc_already_running_test");
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let _ = fs::write(temp_dir.join("Cargo.toml"), "[package]\nname = \"fc_running\"\n");
    let file_path = temp_dir.join("src").join("lib.rs");
    let uri = format!("file://{}", file_path.display());

    let (mut lc, mut mr, mut om) = setup_server_with_doc(&uri, "fn f() {}\n");
    lc.flycheck.trigger_check(&temp_dir);
    assert_eq!(lc.flycheck.current_generation(), 1);

    let (ok, _) = dispatch_did_save(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didSave", "params": { "textDocument": { "uri": uri } } }),
        &mut lc, &mut mr, &mut om,
    );

    let _ = fs::remove_dir_all(&temp_dir);
    assert!(ok);
    assert_eq!(lc.flycheck.current_generation(), 2);
}

#[test]
fn test_did_save_with_root_containing_cargo_workspace() {
    let temp_dir = std::env::temp_dir().join("ws_save_test");
    let _ = fs::create_dir_all(temp_dir.join("crates/member/src"));
    let _ = fs::write(temp_dir.join("Cargo.toml"), "[workspace]\nmembers = [\"crates/member\"]\n");
    let _ = fs::write(temp_dir.join("crates/member/Cargo.toml"), "[package]\nname = \"member\"\n");
    let file_path = temp_dir.join("crates/member/src/lib.rs");
    let uri = format!("file://{}", file_path.display());

    let (mut lc, mut mr, mut om) = setup_server_with_doc(&uri, "pub fn member_fn() {}\n");

    let (ok, _) = dispatch_did_save(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didSave", "params": { "textDocument": { "uri": uri } } }),
        &mut lc, &mut mr, &mut om,
    );

    let _ = fs::remove_dir_all(&temp_dir);
    assert!(ok);
    assert_eq!(lc.flycheck.current_generation(), 1);
}
