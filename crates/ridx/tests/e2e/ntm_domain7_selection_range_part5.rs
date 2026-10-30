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
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static SEL_P5_COUNTER: AtomicU64 = AtomicU64::new(500);

fn to_uri(p: &Path) -> String {
    path_to_uri(p.to_str().unwrap())
}

fn temp_file(tag: &str) -> PathBuf {
    let id = SEL_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("ridx_sel_p5_{}_{}_{}.rs", tag, std::process::id(), id))
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
fn test_selection_range_cursor_on_line_comment_inside_fn() {
    let file = temp_file("lcomment");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "fn with_comment() {\n    // helper comment\n    let x = 5;\n}\n");

    // Inside a comment the whole comment is the innermost node, not a word of it.
    let result = selection_ranges(&mut dsp, 501, &file, &[(1, 8)]);
    assert_eq!(result, json!([chain(&[(1, 4, 1, 21), (0, 18, 3, 1), (0, 0, 3, 1), (0, 0, 4, 0)])]));
}

#[test]
fn test_selection_range_cursor_on_block_comment_inside_fn() {
    let file = temp_file("bcomment");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "fn with_bcomment() {\n    /* block comment\n       spanning lines */\n    let y = 10;\n}\n");

    // A multi-line block comment is one node from `/*` to `*/`.
    let result = selection_ranges(&mut dsp, 502, &file, &[(2, 8)]);
    assert_eq!(result, json!([chain(&[(1, 4, 2, 24), (0, 19, 4, 1), (0, 0, 4, 1), (0, 0, 5, 0)])]));
}

#[test]
fn test_selection_range_cursor_on_leading_whitespace_of_block() {
    let file = temp_file("ws");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "fn ws_test() {\n    {\n        let z = 20;\n    }\n}\n");

    // Indentation carries no token: the inner block is the innermost node.
    let result = selection_ranges(&mut dsp, 503, &file, &[(2, 4)]);
    assert_eq!(result, json!([chain(&[(1, 4, 3, 5), (0, 13, 4, 1), (0, 0, 4, 1), (0, 0, 5, 0)])]));
}

#[test]
fn test_selection_range_cursor_on_opening_brace_boundary() {
    let file = temp_file("opbrace");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "fn brace_open() {\n    let a = 1;\n}\n");

    // Cursor on `{`: the body starts here; `)` before it is not a token.
    let result = selection_ranges(&mut dsp, 504, &file, &[(0, 16)]);
    assert_eq!(result, json!([chain(&[(0, 16, 2, 1), (0, 0, 2, 1), (0, 0, 3, 0)])]));
}

#[test]
fn test_selection_range_cursor_on_closing_brace_boundary() {
    let file = temp_file("clbrace");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "fn brace_close() {\n    let a = 1;\n}\n");

    // Cursor on `}` and right after it: both still inside the body.
    let result = selection_ranges(&mut dsp, 505, &file, &[(2, 0), (2, 1)]);
    let expected = chain(&[(0, 17, 2, 1), (0, 0, 2, 1), (0, 0, 3, 0)]);
    assert_eq!(result, json!([expected, expected]));
}

#[test]
fn test_selection_range_cursor_between_nested_blocks() {
    let file = temp_file("between");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "fn two_blocks() {\n    {\n        let a = 1;\n    }\n    // gap\n    {\n        let b = 2;\n    }\n}\n");

    // In the gap neither block is an ancestor; on the indentation the chain starts at the fn
    // body, on the `// gap` comment it starts at that comment.
    let result = selection_ranges(&mut dsp, 506, &file, &[(4, 2), (4, 5)]);
    assert_eq!(result, json!([chain(&[(0, 16, 8, 1), (0, 0, 8, 1), (0, 0, 9, 0)]), chain(&[(4, 4, 4, 10), (0, 16, 8, 1), (0, 0, 8, 1), (0, 0, 9, 0)]),]));
}
