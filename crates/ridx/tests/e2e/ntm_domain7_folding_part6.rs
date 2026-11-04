/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/foldingRange (Domain 7, Part 6).
//! Tests live overlay scopes, nested overlay ranges, expansion/contraction on didChange, and didClose fallback.

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::path_to_uri;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static FOLD_P6_COUNTER: AtomicU64 = AtomicU64::new(1);

fn to_uri(p: &Path) -> String {
    path_to_uri(p.to_str().unwrap())
}

struct TestDispatcher {
    lifecycle: LifecycleManager,
    multi_root: MultiRootWorkspace,
    overlay_mgr: OverlayManager,
    cancelled: Arc<Mutex<HashSet<Value>>>,
    writer: Vec<u8>,
}

impl TestDispatcher {
    fn new() -> Self {
        let mut d = Self {
            lifecycle: LifecycleManager::new(),
            multi_root: MultiRootWorkspace::new(),
            overlay_mgr: OverlayManager::new(),
            cancelled: Arc::new(Mutex::new(HashSet::new())),
            writer: Vec::new(),
        };
        let init_msg = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } });
        assert!(d.dispatch(&init_msg));
        d.writer.clear();
        d
    }

    fn dispatch(&mut self, msg: &Value) -> bool {
        dispatch_lsp_message(msg.clone(), &mut self.writer, &mut self.lifecycle, &mut self.multi_root, &mut self.overlay_mgr, &self.cancelled, None).unwrap()
    }

    fn parse_response(&mut self) -> Value {
        let text = String::from_utf8(self.writer.clone()).unwrap();
        self.writer.clear();
        let body = text.split("\r\n\r\n").nth(1).unwrap();
        serde_json::from_str(body).unwrap()
    }
}

#[test]
fn test_folding_range_overlay_multiline_scope_emitted() {
    let id = FOLD_P6_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_fold_p6_ovmulti_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("lib.rs");
    std::fs::write(&file, "// on disk\n").unwrap();

    let mut dsp = TestDispatcher::new();
    let did_open = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn live() {\n    let val = 100;\n    val\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&did_open));

    let req = json!({
        "jsonrpc": "2.0", "id": 219, "method": "textDocument/foldingRange",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 219);
    let ranges = resp["result"].as_array().unwrap();
    // The open buffer's text is folded, not the file on disk: the function, as in rust-analyzer (no whole-file fold).
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0]["kind"], "region");
    assert_eq!(ranges[0]["startLine"], 0);
    assert_eq!(ranges[0]["endLine"], 3);
}

#[test]
fn test_folding_range_overlay_nested_scopes() {
    let id = FOLD_P6_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_fold_p6_ovnest_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("lib.rs");
    std::fs::write(&file, "// on disk\n").unwrap();

    let mut dsp = TestDispatcher::new();
    let did_open = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn outer() {\n    {\n        inner();\n    }\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&did_open));

    let req = json!({
        "jsonrpc": "2.0", "id": 220, "method": "textDocument/foldingRange",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 220);
    let ranges = resp["result"].as_array().unwrap();
    // Inner block (1..3), then the outer fn (0..4); no whole-file fold.
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0]["startLine"], 1);
    assert_eq!(ranges[0]["endLine"], 3);
    assert_eq!(ranges[1]["startLine"], 0);
    assert_eq!(ranges[1]["endLine"], 4);
}

#[test]
fn test_folding_range_overlay_did_change_expands_range() {
    let id = FOLD_P6_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_fold_p6_ovexpand_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("lib.rs");
    std::fs::write(&file, "// on disk\n").unwrap();

    let mut dsp = TestDispatcher::new();
    let did_open = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn f() {\n    1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&did_open));

    let did_change = json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": to_uri(&file), "version": 2 },
            "contentChanges": [
                {
                    "range": { "start": { "line": 1, "character": 6 }, "end": { "line": 1, "character": 6 } },
                    "text": "\n    2;\n    3;"
                }
            ]
        }
    });
    assert!(dsp.dispatch(&did_change));

    let req = json!({
        "jsonrpc": "2.0", "id": 221, "method": "textDocument/foldingRange",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 221);
    let ranges = resp["result"].as_array().unwrap();
    // The edited buffer's function grows to 0..4.
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0]["startLine"], 0);
    assert_eq!(ranges[0]["endLine"], 4);
}

#[test]
fn test_folding_range_overlay_did_change_contracts_range() {
    let id = FOLD_P6_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_fold_p6_ovcontract_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("lib.rs");
    std::fs::write(&file, "// on disk\n").unwrap();

    let mut dsp = TestDispatcher::new();
    let did_open = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn f() {\n    1;\n    2;\n    3;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&did_open));

    let did_change = json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": to_uri(&file), "version": 2 },
            "contentChanges": [
                {
                    "range": { "start": { "line": 1, "character": 0 }, "end": { "line": 3, "character": 0 } },
                    "text": ""
                }
            ]
        }
    });
    assert!(dsp.dispatch(&did_change));

    let req = json!({
        "jsonrpc": "2.0", "id": 222, "method": "textDocument/foldingRange",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 222);
    let ranges = resp["result"].as_array().unwrap();
    // The edited buffer's function shrinks to 0..2.
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0]["startLine"], 0);
    assert_eq!(ranges[0]["endLine"], 2);
}

#[test]
fn test_folding_range_overlay_did_close_falls_back_to_disk() {
    let id = FOLD_P6_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_fold_p6_ovclose_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("lib.rs");
    std::fs::write(&file, "fn disk() {\n    99;\n}\n").unwrap();

    let mut dsp = TestDispatcher::new();
    let did_open = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn live() {\n    1;\n    2;\n    3;\n    4;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&did_open));

    let did_close = json!({
        "jsonrpc": "2.0", "method": "textDocument/didClose",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&did_close));

    let req = json!({
        "jsonrpc": "2.0", "id": 223, "method": "textDocument/foldingRange",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 223);
    let ranges = resp["result"].as_array().unwrap();
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0]["startLine"], 0);
    assert_eq!(ranges[0]["endLine"], 2);
}

#[test]
fn test_folding_range_overlay_without_scopes_returns_empty() {
    let id = FOLD_P6_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_fold_p6_ovnoscope_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("lib.rs");
    std::fs::write(&file, "// on disk\n").unwrap();

    let mut dsp = TestDispatcher::new();
    let did_open = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn one_line() { 1 } fn second_line() { 2 }"
            }
        }
    });
    assert!(dsp.dispatch(&did_open));

    let req = json!({
        "jsonrpc": "2.0", "id": 224, "method": "textDocument/foldingRange",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 224);
    assert_eq!(resp["result"], json!([]));
}
