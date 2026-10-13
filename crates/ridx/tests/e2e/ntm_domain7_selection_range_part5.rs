/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/selectionRange (Domain 7, Part 5).
//! Tests edge cases: comments, whitespace, brace boundaries, and gaps between blocks.

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

static SEL_P5_COUNTER: AtomicU64 = AtomicU64::new(500);

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
fn test_selection_range_cursor_on_line_comment_inside_fn() {
    let id = SEL_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_sel_p5_lcomment_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn with_comment() {\n    // helper comment\n    let x = 5;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 501, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 1, "character": 8 }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 501);
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);

    let inner = &results[0];
    assert_eq!(inner["range"]["start"]["line"], 0);
    assert_eq!(inner["range"]["start"]["character"], 3);
    assert_eq!(inner["range"]["end"]["line"], 3);
    assert_eq!(inner["range"]["end"]["character"], 1);

    let root_scope = &inner["parent"];
    assert_eq!(root_scope["range"]["start"]["line"], 0);
    assert_eq!(root_scope["range"]["start"]["character"], 0);
    assert_eq!(root_scope["range"]["end"]["line"], 4);
    assert_eq!(root_scope["range"]["end"]["character"], 0);
    assert_eq!(root_scope["parent"], serde_json::Value::Null);
}

#[test]
fn test_selection_range_cursor_on_block_comment_inside_fn() {
    let id = SEL_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_sel_p5_bcomment_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn with_bcomment() {\n    /* block comment\n       spanning lines */\n    let y = 10;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 502, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 2, "character": 8 }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 502);
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);

    let inner = &results[0];
    assert_eq!(inner["range"]["start"]["line"], 0);
    assert_eq!(inner["range"]["start"]["character"], 3);
    assert_eq!(inner["range"]["end"]["line"], 4);
    assert_eq!(inner["range"]["end"]["character"], 1);
}

#[test]
fn test_selection_range_cursor_on_leading_whitespace_of_block() {
    let id = SEL_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_sel_p5_ws_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn ws_test() {\n    {\n        let z = 20;\n    }\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 503, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 2, "character": 4 }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 503);
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);

    let inner = &results[0];
    assert_eq!(inner["range"]["start"]["line"], 1);
    assert_eq!(inner["range"]["start"]["character"], 4);
    assert_eq!(inner["range"]["end"]["line"], 3);
    assert_eq!(inner["range"]["end"]["character"], 5);

    let fn_scope = &inner["parent"];
    assert_eq!(fn_scope["range"]["start"]["line"], 0);
    assert_eq!(fn_scope["range"]["start"]["character"], 3);
    assert_eq!(fn_scope["range"]["end"]["line"], 4);
    assert_eq!(fn_scope["range"]["end"]["character"], 1);
}

#[test]
fn test_selection_range_cursor_on_opening_brace_boundary() {
    let id = SEL_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_sel_p5_opbrace_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn brace_open() {\n    let a = 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 504, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 0, "character": 16 }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 504);
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);

    let inner = &results[0];
    assert_eq!(inner["range"]["start"]["line"], 0);
    assert_eq!(inner["range"]["start"]["character"], 3);
    assert_eq!(inner["range"]["end"]["line"], 2);
    assert_eq!(inner["range"]["end"]["character"], 1);
}

#[test]
fn test_selection_range_cursor_on_closing_brace_boundary() {
    let id = SEL_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_sel_p5_clbrace_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn brace_close() {\n    let a = 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 505, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 2, "character": 0 }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 505);
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);

    let inner = &results[0];
    assert_eq!(inner["range"]["start"]["line"], 0);
    assert_eq!(inner["range"]["start"]["character"], 3);
    assert_eq!(inner["range"]["end"]["line"], 2);
    assert_eq!(inner["range"]["end"]["character"], 1);
}

#[test]
fn test_selection_range_cursor_between_nested_blocks() {
    let id = SEL_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_sel_p5_between_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn two_blocks() {\n    {\n        let a = 1;\n    }\n    // gap\n    {\n        let b = 2;\n    }\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 506, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 4, "character": 5 }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 506);
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);

    // Cursor is in the gap outside block 1 and block 2, so innermost scope is two_blocks!
    let inner = &results[0];
    assert_eq!(inner["range"]["start"]["line"], 0);
    assert_eq!(inner["range"]["start"]["character"], 3);
    assert_eq!(inner["range"]["end"]["line"], 8);
    assert_eq!(inner["range"]["end"]["character"], 1);

    let root_scope = &inner["parent"];
    assert_eq!(root_scope["range"]["start"]["line"], 0);
    assert_eq!(root_scope["range"]["start"]["character"], 0);
    assert_eq!(root_scope["range"]["end"]["line"], 9);
    assert_eq!(root_scope["range"]["end"]["character"], 0);
    assert_eq!(root_scope["parent"], serde_json::Value::Null);
}
