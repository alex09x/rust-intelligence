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
use serde_json::json;
use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static SEL_P7_COUNTER: AtomicU64 = AtomicU64::new(700);

fn to_uri(p: &Path) -> String {
    path_to_uri(p.to_str().unwrap())
}

struct TestDispatcher {
    lifecycle: LifecycleManager,
    multi_root: MultiRootWorkspace,
    overlay_mgr: OverlayManager,
    cancelled: Arc<Mutex<HashSet<serde_json::Value>>>,
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

    fn dispatch(&mut self, msg: &serde_json::Value) -> bool {
        dispatch_lsp_message(
            msg.clone(), &mut self.writer, &mut self.lifecycle,
            &mut self.multi_root, &mut self.overlay_mgr, &self.cancelled, None,
        ).unwrap()
    }

    fn parse_response(&mut self) -> serde_json::Value {
        let text = String::from_utf8(self.writer.clone()).unwrap();
        self.writer.clear();
        let body = text.split("\r\n\r\n").nth(1).unwrap();
        serde_json::from_str(body).unwrap()
    }
}

#[test]
fn test_selection_range_overlay_did_close_falls_back_to_empty() {
    let id = SEL_P7_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_sel_p7_close_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn close_test() {\n    let active = true;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req1 = json!({
        "jsonrpc": "2.0", "id": 701, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 1, "character": 8 }]
        }
    });
    assert!(dsp.dispatch(&req1));
    let resp1 = dsp.parse_response();
    assert_eq!(resp1["id"], 701);
    assert_eq!(resp1["result"].as_array().unwrap().len(), 1);

    // Send didClose
    let close_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didClose",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&close_msg));

    // Query again - should be empty since overlay is closed
    let req2 = json!({
        "jsonrpc": "2.0", "id": 702, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 1, "character": 8 }]
        }
    });
    assert!(dsp.dispatch(&req2));
    let resp2 = dsp.parse_response();
    assert_eq!(resp2["id"], 702);
    assert_eq!(resp2["result"].as_array().unwrap().len(), 0);
}

#[test]
fn test_selection_range_did_change_nested_block_added() {
    let id = SEL_P7_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_sel_p7_addblock_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn dynamic() {\n    let a = 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    // Insert block inside dynamic
    let change_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": to_uri(&file), "version": 2 },
            "contentChanges": [
                {
                    "range": { "start": { "line": 1, "character": 4 }, "end": { "line": 1, "character": 14 } },
                    "text": "{\n        let nested = 10;\n    }"
                }
            ]
        }
    });
    assert!(dsp.dispatch(&change_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 703, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 2, "character": 12 }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 703);
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);

    // Inner block range
    let inner = &results[0];
    assert_eq!(inner["range"]["start"]["line"], 1);
    assert_eq!(inner["range"]["start"]["character"], 4);
    assert_eq!(inner["range"]["end"]["line"], 3);
    assert_eq!(inner["range"]["end"]["character"], 5);

    // Parent dynamic fn scope
    let fn_scope = &inner["parent"];
    assert_eq!(fn_scope["range"]["start"]["line"], 0);
    assert_eq!(fn_scope["range"]["start"]["character"], 3);
    assert_eq!(fn_scope["range"]["end"]["line"], 4);
    assert_eq!(fn_scope["range"]["end"]["character"], 1);
}

#[test]
fn test_selection_range_did_change_block_removed() {
    let id = SEL_P7_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_sel_p7_delblock_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn block_del() {\n    {\n        let inner = 1;\n    }\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    // Replace lines 1..3 with flat statement
    let change_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": to_uri(&file), "version": 2 },
            "contentChanges": [
                {
                    "range": { "start": { "line": 1, "character": 4 }, "end": { "line": 3, "character": 5 } },
                    "text": "let flat = 2;"
                }
            ]
        }
    });
    assert!(dsp.dispatch(&change_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 704, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 1, "character": 8 }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 704);
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);

    // Inner scope is now directly block_del fn
    let inner = &results[0];
    assert_eq!(inner["range"]["start"]["line"], 0);
    assert_eq!(inner["range"]["start"]["character"], 3);
    assert_eq!(inner["range"]["end"]["line"], 2);
    assert_eq!(inner["range"]["end"]["character"], 1);
    assert_eq!(inner["parent"]["range"]["start"]["line"], 0);
    assert_eq!(inner["parent"]["range"]["start"]["character"], 0);
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
    assert_eq!(resp["result"].as_array().unwrap().len(), 0);
}

#[test]
fn test_selection_range_null_positions_treated_safely() {
    let id = SEL_P7_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_sel_p7_nullpos_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn dummy() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 706, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": null
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 706);
    assert_eq!(resp["result"].as_array().unwrap().len(), 0);
}

#[test]
fn test_selection_range_consecutive_requests_idempotent() {
    let id = SEL_P7_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_sel_p7_idempotent_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn idem() {\n    let val = 99;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req1 = json!({
        "jsonrpc": "2.0", "id": 707, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 1, "character": 8 }]
        }
    });
    assert!(dsp.dispatch(&req1));
    let resp1 = dsp.parse_response();
    assert_eq!(resp1["id"], 707);

    let req2 = json!({
        "jsonrpc": "2.0", "id": 708, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 1, "character": 8 }]
        }
    });
    assert!(dsp.dispatch(&req2));
    let resp2 = dsp.parse_response();
    assert_eq!(resp2["id"], 708);

    assert_eq!(resp1["result"], resp2["result"]);
}
