/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 3: `willSave` / `willSaveWaitUntil` tests (part 3: cancels, sequences, and converter caches).

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::uri_to_path;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::Cursor;
use std::sync::{Arc, Mutex};

fn extract_response(out: &[u8]) -> Value {
    let s = String::from_utf8_lossy(out);
    for part in s.split("Content-Length: ") {
        if let Some(idx) = part.find("\r\n\r\n") {
            let body = &part[idx + 4..];
            if let Ok(v) = serde_json::from_str::<Value>(body.trim()) {
                return v;
            }
        }
    }
    Value::Null
}

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

#[test]
fn test_will_save_wait_until_cancelled_request_returns_request_cancelled_error() {
    let uri = "file:///tmp/cancelled_wait.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");
    let cancelled = Arc::new(Mutex::new(HashSet::new()));
    let req_id = 999;
    cancelled.lock().unwrap().insert(json!(req_id));

    let mut out = Cursor::new(Vec::new());
    let ok = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "id": req_id,
            "method": "textDocument/willSaveWaitUntil",
            "params": { "textDocument": { "uri": uri }, "reason": 1 }
        }),
        &mut out,
        &mut lc,
        &mut mr,
        &mut om,
        &cancelled,
        None,
    )
    .unwrap_or(false);

    assert!(ok);
    let resp = extract_response(&out.into_inner());
    assert_eq!(resp["id"], req_id);
    assert_eq!(resp["error"]["code"], -32800);
    assert_eq!(resp["error"]["message"], "Request cancelled");
}

#[test]
fn test_will_save_followed_by_did_save_sequence() {
    let uri = "file:///tmp/seq_save.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");
    let cancelled = Arc::new(Mutex::new(HashSet::new()));
    let file = uri_to_path(uri);

    let mut out1 = Cursor::new(Vec::new());
    let ok1 = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/willSave",
            "params": { "textDocument": { "uri": uri }, "reason": 1 }
        }),
        &mut out1,
        &mut lc,
        &mut mr,
        &mut om,
        &cancelled,
        None,
    )
    .unwrap_or(false);
    assert!(ok1);

    let mut out2 = Cursor::new(Vec::new());
    let ok2 = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": uri } }
        }),
        &mut out2,
        &mut lc,
        &mut mr,
        &mut om,
        &cancelled,
        None,
    )
    .unwrap_or(false);
    assert!(ok2);

    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "fn f() {}\n");
}

#[test]
fn test_will_save_wait_until_followed_by_did_save() {
    let uri = "file:///tmp/wait_then_save.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let mut out1 = Cursor::new(Vec::new());
    let ok1 = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "textDocument/willSaveWaitUntil",
            "params": { "textDocument": { "uri": uri }, "reason": 1 }
        }),
        &mut out1,
        &mut lc,
        &mut mr,
        &mut om,
        &cancelled,
        None,
    )
    .unwrap_or(false);
    assert!(ok1);
    let resp1 = extract_response(&out1.into_inner());
    assert_eq!(resp1["result"], json!([]));

    let mut out2 = Cursor::new(Vec::new());
    let ok2 = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": uri } }
        }),
        &mut out2,
        &mut lc,
        &mut mr,
        &mut om,
        &cancelled,
        None,
    )
    .unwrap_or(false);
    assert!(ok2);
}

#[test]
fn test_will_save_multiple_successive_notifications() {
    let uri = "file:///tmp/multi_will_save.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    for reason in 1..=3 {
        let mut out = Cursor::new(Vec::new());
        let ok = dispatch_lsp_message(
            json!({
                "jsonrpc": "2.0",
                "method": "textDocument/willSave",
                "params": { "textDocument": { "uri": uri }, "reason": reason }
            }),
            &mut out,
            &mut lc,
            &mut mr,
            &mut om,
            &cancelled,
            None,
        )
        .unwrap_or(false);
        assert!(ok);
        assert_eq!(out.into_inner().len(), 0);
    }
}

#[test]
fn test_will_save_wait_until_multiple_successive_requests() {
    let uri = "file:///tmp/multi_wait_until.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    for req_id in 101..=103 {
        let mut out = Cursor::new(Vec::new());
        let ok = dispatch_lsp_message(
            json!({
                "jsonrpc": "2.0",
                "id": req_id,
                "method": "textDocument/willSaveWaitUntil",
                "params": { "textDocument": { "uri": uri }, "reason": 1 }
            }),
            &mut out,
            &mut lc,
            &mut mr,
            &mut om,
            &cancelled,
            None,
        )
        .unwrap_or(false);
        assert!(ok);
        let resp = extract_response(&out.into_inner());
        assert_eq!(resp["id"], req_id);
        assert_eq!(resp["result"], json!([]));
    }
}

#[test]
fn test_will_save_with_dirty_overlay_preserves_dirty_state() {
    let uri = "file:///tmp/dirty_will_save.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "initial");
    let file = uri_to_path(uri);
    om.apply_content_changes(&file, 2, &[json!({ "text": "dirty content" })]);
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let mut out = Cursor::new(Vec::new());
    let ok = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/willSave",
            "params": { "textDocument": { "uri": uri }, "reason": 1 }
        }),
        &mut out,
        &mut lc,
        &mut mr,
        &mut om,
        &cancelled,
        None,
    )
    .unwrap_or(false);

    assert!(ok);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "dirty content");
    assert_eq!(overlay.version, 2);
}

#[test]
fn test_will_save_wait_until_preserves_symbols_count() {
    let uri = "file:///tmp/sym_wait.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "pub fn a() {}\npub fn b() {}\n");
    let cancelled = Arc::new(Mutex::new(HashSet::new()));
    let file = uri_to_path(uri);

    let mut out = Cursor::new(Vec::new());
    let ok = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "textDocument/willSaveWaitUntil",
            "params": { "textDocument": { "uri": uri }, "reason": 1 }
        }),
        &mut out,
        &mut lc,
        &mut mr,
        &mut om,
        &cancelled,
        None,
    )
    .unwrap_or(false);

    assert!(ok);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.symbols.len(), 2);
}

#[test]
fn test_will_save_does_not_invalidate_position_converter() {
    let temp_dir = std::env::temp_dir().join(format!("conv_ws_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let path = temp_dir.join("conv.rs");
    let _ = std::fs::write(&path, "line1_initial\n");
    let uri = format!("file://{}", path.display());
    let (mut lc, mut mr, mut om) = setup_server_with_doc(&uri, "line1_initial\n");
    let file = uri_to_path(&uri);
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    // Warm up converter cache
    let cached = lc.conv.get_line(&file, 1);
    assert_eq!(cached, Some("line1_initial"));

    // Write updated content to disk behind converter cache
    let _ = std::fs::write(&path, "line1_disk_updated\n");
    assert_eq!(lc.conv.get_line(&file, 1), Some("line1_initial"));

    let mut out = Cursor::new(Vec::new());
    let ok = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/willSave",
            "params": { "textDocument": { "uri": uri }, "reason": 1 }
        }),
        &mut out,
        &mut lc,
        &mut mr,
        &mut om,
        &cancelled,
        None,
    )
    .unwrap_or(false);

    assert!(ok);
    // willSave is a no-op that does NOT invalidate converter cache, so old cached line remains
    assert_eq!(lc.conv.get_line(&file, 1), Some("line1_initial"));
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_will_save_wait_until_does_not_invalidate_position_converter() {
    let temp_dir = std::env::temp_dir().join(format!("conv_wsu_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let path = temp_dir.join("conv.rs");
    let _ = std::fs::write(&path, "line1_initial\n");
    let uri = format!("file://{}", path.display());
    let (mut lc, mut mr, mut om) = setup_server_with_doc(&uri, "line1_initial\n");
    let file = uri_to_path(&uri);
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    // Warm up converter cache
    let cached = lc.conv.get_line(&file, 1);
    assert_eq!(cached, Some("line1_initial"));

    // Write updated content to disk behind converter cache
    let _ = std::fs::write(&path, "line1_disk_updated\n");
    assert_eq!(lc.conv.get_line(&file, 1), Some("line1_initial"));

    let mut out = Cursor::new(Vec::new());
    let ok = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "textDocument/willSaveWaitUntil",
            "params": { "textDocument": { "uri": uri }, "reason": 1 }
        }),
        &mut out,
        &mut lc,
        &mut mr,
        &mut om,
        &cancelled,
        None,
    )
    .unwrap_or(false);

    assert!(ok);
    assert_eq!(lc.conv.get_line(&file, 1), Some("line1_initial"));
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_will_save_does_not_trigger_flycheck() {
    let uri = "file:///tmp/fc_will_save.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let mut out = Cursor::new(Vec::new());
    let ok = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/willSave",
            "params": { "textDocument": { "uri": uri }, "reason": 1 }
        }),
        &mut out,
        &mut lc,
        &mut mr,
        &mut om,
        &cancelled,
        None,
    )
    .unwrap_or(false);

    assert!(ok);
    assert_eq!(lc.flycheck.current_generation(), 0);
}
