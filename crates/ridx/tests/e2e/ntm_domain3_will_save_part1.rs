/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 3: `textDocument/willSave` notification tests (part 1: reasons, safety, and invariants).

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
    let mut lc = LifecycleManager::new();
    let init = json!({ "params": { "capabilities": {} } });
    let _ = lc.handle_initialize(&init);
    let mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();
    let file = uri_to_path(uri);
    om.open_with_provenance(uri, &file, 1, text.to_string(), true);
    (lc, mr, om)
}

fn dispatch_will_save(msg: Value, lifecycle: &mut LifecycleManager, multi_root: &mut MultiRootWorkspace, overlay_mgr: &mut OverlayManager) -> (bool, Vec<u8>) {
    let mut out = Cursor::new(Vec::new());
    let cancelled = Arc::new(Mutex::new(HashSet::new()));
    let res = dispatch_lsp_message(msg, &mut out, lifecycle, multi_root, overlay_mgr, &cancelled, None);
    (res.unwrap_or(false), out.into_inner())
}

#[test]
fn test_will_save_manual_reason_returns_ok_true() {
    let uri = "file:///tmp/will_save_manual.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn main() {}\n");

    let (ok, out) = dispatch_will_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/willSave",
            "params": {
                "textDocument": { "uri": uri },
                "reason": 1
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert!(ok);
    assert_eq!(out.len(), 0);
}

#[test]
fn test_will_save_after_delay_reason_returns_ok_true() {
    let uri = "file:///tmp/will_save_delay.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn main() {}\n");

    let (ok, out) = dispatch_will_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/willSave",
            "params": {
                "textDocument": { "uri": uri },
                "reason": 2
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert!(ok);
    assert_eq!(out.len(), 0);
}

#[test]
fn test_will_save_focus_out_reason_returns_ok_true() {
    let uri = "file:///tmp/will_save_focus.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn main() {}\n");

    let (ok, out) = dispatch_will_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/willSave",
            "params": {
                "textDocument": { "uri": uri },
                "reason": 3
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert!(ok);
    assert_eq!(out.len(), 0);
}

#[test]
fn test_will_save_unspecified_reason_handled_safely() {
    let uri = "file:///tmp/will_save_noreason.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn main() {}\n");

    let (ok, out) = dispatch_will_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/willSave",
            "params": {
                "textDocument": { "uri": uri }
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert!(ok);
    assert_eq!(out.len(), 0);
}

#[test]
fn test_will_save_unknown_reason_code_handled_safely() {
    let uri = "file:///tmp/will_save_unknown_code.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn main() {}\n");

    let (ok, out) = dispatch_will_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/willSave",
            "params": {
                "textDocument": { "uri": uri },
                "reason": 999
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert!(ok);
    assert_eq!(out.len(), 0);
}

#[test]
fn test_will_save_does_not_modify_open_overlay_content() {
    let uri = "file:///tmp/will_save_content.rs";
    let initial_text = "pub fn preserved() -> bool { true }\n";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, initial_text);
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_will_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/willSave",
            "params": {
                "textDocument": { "uri": uri },
                "reason": 1
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert!(ok);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, initial_text);
}

#[test]
fn test_will_save_does_not_modify_open_overlay_version() {
    let uri = "file:///tmp/will_save_version.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_will_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/willSave",
            "params": {
                "textDocument": { "uri": uri },
                "reason": 1
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert!(ok);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.version, 1);
}

#[test]
fn test_will_save_on_unopened_document_is_safe_noop() {
    let mut lc = LifecycleManager::new();
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();
    let uri = "file:///tmp/unopened_will_save.rs";

    let (ok, out) = dispatch_will_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/willSave",
            "params": {
                "textDocument": { "uri": uri },
                "reason": 1
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert!(ok);
    assert_eq!(out.len(), 0);
    assert_eq!(om.open_count(), 0);
}

#[test]
fn test_will_save_empty_params_handled_safely() {
    let mut lc = LifecycleManager::new();
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();

    let (ok, out) = dispatch_will_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/willSave",
            "params": {}
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert!(ok);
    assert_eq!(out.len(), 0);
}

#[test]
fn test_will_save_produces_zero_stdout_output() {
    let uri = "file:///tmp/zero_out.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "let x = 10;\n");

    let (ok, out) = dispatch_will_save(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/willSave",
            "params": {
                "textDocument": { "uri": uri },
                "reason": 2
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
    );

    assert!(ok);
    assert_eq!(out.len(), 0);
}
