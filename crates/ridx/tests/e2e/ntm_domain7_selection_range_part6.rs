/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/selectionRange (Domain 7, Part 6).
//! Tests batch positions, order preservation, CRLF, UTF-8, and edit coordinate shifts.

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

static SEL_P6_COUNTER: AtomicU64 = AtomicU64::new(600);

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
fn test_selection_range_batch_three_positions_different_functions() {
    let id = SEL_P6_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_sel_p6_batch3_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn f1() {\n    let a = 1;\n}\nfn f2() {\n    let b = 2;\n}\nfn f3() {\n    let c = 3;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 601, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [
                { "line": 1, "character": 8 },
                { "line": 4, "character": 8 },
                { "line": 7, "character": 8 }
            ]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 601);
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 3);

    assert_eq!(results[0]["range"]["start"]["line"], 0);
    assert_eq!(results[0]["range"]["start"]["character"], 3);
    assert_eq!(results[0]["range"]["end"]["line"], 2);
    assert_eq!(results[0]["range"]["end"]["character"], 1);

    assert_eq!(results[1]["range"]["start"]["line"], 3);
    assert_eq!(results[1]["range"]["start"]["character"], 3);
    assert_eq!(results[1]["range"]["end"]["line"], 5);
    assert_eq!(results[1]["range"]["end"]["character"], 1);

    assert_eq!(results[2]["range"]["start"]["line"], 6);
    assert_eq!(results[2]["range"]["start"]["character"], 3);
    assert_eq!(results[2]["range"]["end"]["line"], 8);
    assert_eq!(results[2]["range"]["end"]["character"], 1);
}

#[test]
fn test_selection_range_batch_positions_in_same_function_different_scopes() {
    let id = SEL_P6_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_sel_p6_samediff_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn multi_scope() {\n    {\n        let a = 1;\n    }\n    {\n        let b = 2;\n    }\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 602, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [
                { "line": 2, "character": 8 },
                { "line": 5, "character": 8 }
            ]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 602);
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 2);

    // Block A
    assert_eq!(results[0]["range"]["start"]["line"], 1);
    assert_eq!(results[0]["range"]["start"]["character"], 4);
    assert_eq!(results[0]["range"]["end"]["line"], 3);
    assert_eq!(results[0]["range"]["end"]["character"], 5);

    // Block B
    assert_eq!(results[1]["range"]["start"]["line"], 4);
    assert_eq!(results[1]["range"]["start"]["character"], 4);
    assert_eq!(results[1]["range"]["end"]["line"], 6);
    assert_eq!(results[1]["range"]["end"]["character"], 5);

    // Both share multi_scope as parent
    assert_eq!(results[0]["parent"]["range"]["start"]["line"], 0);
    assert_eq!(results[0]["parent"]["range"]["start"]["character"], 3);
    assert_eq!(results[1]["parent"]["range"]["start"]["line"], 0);
    assert_eq!(results[1]["parent"]["range"]["start"]["character"], 3);
}

#[test]
fn test_selection_range_batch_positions_ordered_descending() {
    let id = SEL_P6_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_sel_p6_desc_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn alpha() {\n    let x = 1;\n}\nfn omega() {\n    let y = 2;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 603, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [
                { "line": 4, "character": 8 },
                { "line": 1, "character": 8 }
            ]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 603);
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 2);

    // First position was in omega (lines 3..5)
    assert_eq!(results[0]["range"]["start"]["line"], 3);
    assert_eq!(results[0]["range"]["start"]["character"], 3);
    assert_eq!(results[0]["range"]["end"]["line"], 5);
    assert_eq!(results[0]["range"]["end"]["character"], 1);

    // Second position was in alpha (lines 0..2)
    assert_eq!(results[1]["range"]["start"]["line"], 0);
    assert_eq!(results[1]["range"]["start"]["character"], 3);
    assert_eq!(results[1]["range"]["end"]["line"], 2);
    assert_eq!(results[1]["range"]["end"]["character"], 1);
}

#[test]
fn test_selection_range_crlf_line_endings_position_calculation() {
    let id = SEL_P6_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_sel_p6_crlf_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn crlf_fn() {\r\n    let a = 100;\r\n}\r\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 604, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 1, "character": 8 }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 604);
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);

    let inner = &results[0];
    assert_eq!(inner["range"]["start"]["line"], 0);
    assert_eq!(inner["range"]["start"]["character"], 3);
    assert_eq!(inner["range"]["end"]["line"], 2);
    assert_eq!(inner["range"]["end"]["character"], 1);
}

#[test]
fn test_selection_range_utf8_multibyte_prefix_columns() {
    let id = SEL_P6_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_sel_p6_utf8_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn utf8_fn() {\n    // Γειάσα σας\n    let val = 42;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 605, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 2, "character": 8 }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 605);
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);

    let inner = &results[0];
    assert_eq!(inner["range"]["start"]["line"], 0);
    assert_eq!(inner["range"]["start"]["character"], 3);
    assert_eq!(inner["range"]["end"]["line"], 3);
    assert_eq!(inner["range"]["end"]["character"], 1);
}

#[test]
fn test_selection_range_overlay_did_change_shifts_selection_range() {
    let id = SEL_P6_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_sel_p6_shift_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn shifted() {\n    let k = 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    // Prepend 2 lines
    let change_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": to_uri(&file), "version": 2 },
            "contentChanges": [
                {
                    "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
                    "text": "// comment 1\n// comment 2\n"
                }
            ]
        }
    });
    assert!(dsp.dispatch(&change_msg));

    // Now query at line 3 (which was line 1 before shift)
    let req = json!({
        "jsonrpc": "2.0", "id": 606, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 3, "character": 8 }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 606);
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);

    let inner = &results[0];
    assert_eq!(inner["range"]["start"]["line"], 2);
    assert_eq!(inner["range"]["start"]["character"], 3);
    assert_eq!(inner["range"]["end"]["line"], 4);
    assert_eq!(inner["range"]["end"]["character"], 1);
}
