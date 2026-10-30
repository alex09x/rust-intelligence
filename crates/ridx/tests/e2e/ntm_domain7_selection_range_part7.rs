/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/selectionRange (Domain 7, Part 7).
//! Tests mutation lifecycle, didClose, dynamic block insertion/deletion, invalid URI, null positions, and idempotency.

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::path_to_uri;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static SEL_P7_COUNTER: AtomicU64 = AtomicU64::new(700);

fn to_uri(p: &Path) -> String {
    path_to_uri(p.to_str().unwrap())
}

fn temp_file(tag: &str) -> PathBuf {
    let id = SEL_P7_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("ridx_sel_p7_{}_{}_{}.rs", tag, std::process::id(), id))
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

/// Expected chain, innermost first, from `(start_line, start_char, end_line, end_char)`.
fn chain(ranges: &[(u32, u32, u32, u32)]) -> Value {
    let mut current = Value::Null;
    for &(sl, sc, el, ec) in ranges.iter().rev() {
        current = json!({
            "range": { "start": { "line": sl, "character": sc }, "end": { "line": el, "character": ec } },
            "parent": current
        });
    }
    current
}

fn open_doc(dsp: &mut TestDispatcher, file: &Path, text: &str) {
    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": { "uri": to_uri(file), "languageId": "rust", "version": 1, "text": text } }
    });
    assert!(dsp.dispatch(&open_msg));
}

fn selection_ranges(dsp: &mut TestDispatcher, id: u64, file: &Path, positions: &[(u32, u32)]) -> Value {
    let positions: Vec<Value> = positions.iter().map(|&(l, c)| json!({ "line": l, "character": c })).collect();
    let req = json!({
        "jsonrpc": "2.0", "id": id, "method": "textDocument/selectionRange",
        "params": { "textDocument": { "uri": to_uri(file) }, "positions": positions }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], id);
    resp["result"].clone()
}

#[test]
fn test_selection_range_overlay_did_close_falls_back_to_empty() {
    let file = temp_file("close");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "fn close_test() {\n    let active = true;\n}\n");

    let result = selection_ranges(&mut dsp, 701, &file, &[(1, 8)]);
    assert_eq!(result, json!([chain(&[(1, 8, 1, 14), (0, 16, 2, 1), (0, 0, 2, 1), (0, 0, 3, 0)])]));

    let close_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didClose",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&close_msg));

    // The buffer is gone and the file was never written to disk: nothing to select in.
    assert_eq!(selection_ranges(&mut dsp, 702, &file, &[(1, 8)]), json!([]));
}

#[test]
fn test_selection_range_did_change_nested_block_added() {
    let file = temp_file("addblock");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "fn dynamic() {\n    let a = 1;\n}\n");

    // Replace `let a = 1;` with a nested block.
    let change_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": to_uri(&file), "version": 2 },
            "contentChanges": [{
                "range": { "start": { "line": 1, "character": 4 }, "end": { "line": 1, "character": 14 } },
                "text": "{\n        let nested = 10;\n    }"
            }]
        }
    });
    assert!(dsp.dispatch(&change_msg));

    let result = selection_ranges(&mut dsp, 703, &file, &[(2, 12)]);
    assert_eq!(result, json!([chain(&[(2, 12, 2, 18), (1, 4, 3, 5), (0, 13, 4, 1), (0, 0, 4, 1), (0, 0, 5, 0)])]));
}

#[test]
fn test_selection_range_did_change_block_removed() {
    let file = temp_file("delblock");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "fn block_del() {\n    {\n        let inner = 1;\n    }\n}\n");

    // Replace the nested block with a flat statement.
    let change_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": to_uri(&file), "version": 2 },
            "contentChanges": [{
                "range": { "start": { "line": 1, "character": 4 }, "end": { "line": 3, "character": 5 } },
                "text": "let flat = 2;"
            }]
        }
    });
    assert!(dsp.dispatch(&change_msg));

    let result = selection_ranges(&mut dsp, 704, &file, &[(1, 8)]);
    assert_eq!(result, json!([chain(&[(1, 8, 1, 12), (0, 15, 2, 1), (0, 0, 2, 1), (0, 0, 3, 0)])]));
}

#[test]
fn test_selection_range_invalid_uri_format_returns_empty() {
    let mut dsp = TestDispatcher::new();

    let req = json!({
        "jsonrpc": "2.0", "id": 705, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": "not-a-valid-uri" },
            "positions": [{ "line": 0, "character": 0 }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 705);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_selection_range_null_positions_treated_safely() {
    let file = temp_file("nullpos");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "fn dummy() {}\n");

    let req = json!({
        "jsonrpc": "2.0", "id": 706, "method": "textDocument/selectionRange",
        "params": { "textDocument": { "uri": to_uri(&file) }, "positions": null }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 706);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_selection_range_consecutive_requests_idempotent() {
    let file = temp_file("idempotent");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "fn idem() {\n    let val = 99;\n}\n");

    let expected = json!([chain(&[(1, 8, 1, 11), (0, 10, 2, 1), (0, 0, 2, 1), (0, 0, 3, 0)])]);
    assert_eq!(selection_ranges(&mut dsp, 707, &file, &[(1, 8)]), expected);
    assert_eq!(selection_ranges(&mut dsp, 708, &file, &[(1, 8)]), expected);
}
