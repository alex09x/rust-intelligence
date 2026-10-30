/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/selectionRange (Domain 7, Part 3).
//! Tests control flow and block constructs: match, for, while, if/else, let-block, unsafe.

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

static SEL_P3_COUNTER: AtomicU64 = AtomicU64::new(300);

fn to_uri(p: &Path) -> String {
    path_to_uri(p.to_str().unwrap())
}

fn temp_file(tag: &str) -> PathBuf {
    let id = SEL_P3_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("ridx_sel_p3_{}_{}_{}.rs", tag, std::process::id(), id))
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
fn test_selection_range_cursor_inside_match_arm_block() {
    let file = temp_file("match");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "fn eval(x: i32) {\n    match x {\n        1 => {\n            do_one();\n        }\n        _ => (),\n    }\n}\n");

    // `do_one`, arm block, match block, fn body, fn item, file.
    let result = selection_ranges(&mut dsp, 301, &file, &[(3, 14)]);
    assert_eq!(result, json!([chain(&[(3, 12, 3, 18), (2, 13, 4, 9), (1, 12, 6, 5), (0, 16, 7, 1), (0, 0, 7, 1), (0, 0, 8, 0)])]));
    assert_eq!(result[0]["parent"]["parent"]["parent"]["parent"]["parent"]["parent"], Value::Null);
}

#[test]
fn test_selection_range_cursor_inside_for_loop_block() {
    let file = temp_file("for");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "fn loop_test() {\n    for i in 0..10 {\n        let square = i * i;\n    }\n}\n");

    let result = selection_ranges(&mut dsp, 302, &file, &[(2, 12)]);
    assert_eq!(result, json!([chain(&[(2, 12, 2, 18), (1, 19, 3, 5), (0, 15, 4, 1), (0, 0, 4, 1), (0, 0, 5, 0)])]));
}

#[test]
fn test_selection_range_cursor_inside_while_block() {
    let file = temp_file("while");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "fn while_test() {\n    while true {\n        step();\n    }\n}\n");

    let result = selection_ranges(&mut dsp, 303, &file, &[(2, 10)]);
    assert_eq!(result, json!([chain(&[(2, 8, 2, 12), (1, 15, 3, 5), (0, 16, 4, 1), (0, 0, 4, 1), (0, 0, 5, 0)])]));
}

#[test]
fn test_selection_range_cursor_inside_if_else_blocks() {
    let file = temp_file("ifelse");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "fn branch(cond: bool) {\n    if cond {\n        do_then();\n    } else {\n        do_else();\n    }\n}\n");

    // Cursor in the else branch: only the else block is an ancestor, not the if block.
    let result = selection_ranges(&mut dsp, 304, &file, &[(4, 10)]);
    assert_eq!(result, json!([chain(&[(4, 8, 4, 15), (3, 11, 5, 5), (0, 22, 6, 1), (0, 0, 6, 1), (0, 0, 7, 0)])]));
}

#[test]
fn test_selection_range_cursor_inside_block_expression() {
    let file = temp_file("blockexpr");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "fn block_run() {\n    let block = {\n        perform();\n    };\n}\n");

    let result = selection_ranges(&mut dsp, 305, &file, &[(2, 10)]);
    assert_eq!(result, json!([chain(&[(2, 8, 2, 15), (1, 16, 3, 5), (0, 15, 4, 1), (0, 0, 4, 1), (0, 0, 5, 0)])]));
}

#[test]
fn test_selection_range_cursor_inside_unsafe_block() {
    let file = temp_file("unsafe");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "fn unsafe_call() {\n    unsafe {\n        ptr_op();\n    }\n}\n");

    let result = selection_ranges(&mut dsp, 306, &file, &[(2, 10)]);
    assert_eq!(result, json!([chain(&[(2, 8, 2, 14), (1, 11, 3, 5), (0, 17, 4, 1), (0, 0, 4, 1), (0, 0, 5, 0)])]));
}
