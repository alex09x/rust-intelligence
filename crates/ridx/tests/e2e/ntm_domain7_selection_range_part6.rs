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
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static SEL_P6_COUNTER: AtomicU64 = AtomicU64::new(600);

fn to_uri(p: &Path) -> String {
    path_to_uri(p.to_str().unwrap())
}

fn temp_file(tag: &str) -> PathBuf {
    let id = SEL_P6_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("ridx_sel_p6_{}_{}_{}.rs", tag, std::process::id(), id))
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
fn test_selection_range_batch_three_positions_different_functions() {
    let file = temp_file("batch3");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "fn f1() {\n    let a = 1;\n}\nfn f2() {\n    let b = 2;\n}\nfn f3() {\n    let c = 3;\n}\n");

    let result = selection_ranges(&mut dsp, 601, &file, &[(1, 8), (4, 8), (7, 8)]);
    assert_eq!(
        result,
        json!([
            chain(&[(1, 8, 1, 9), (0, 8, 2, 1), (0, 0, 2, 1), (0, 0, 9, 0)]),
            chain(&[(4, 8, 4, 9), (3, 8, 5, 1), (3, 0, 5, 1), (0, 0, 9, 0)]),
            chain(&[(7, 8, 7, 9), (6, 8, 8, 1), (6, 0, 8, 1), (0, 0, 9, 0)]),
        ])
    );
}

#[test]
fn test_selection_range_batch_positions_in_same_function_different_scopes() {
    let file = temp_file("samediff");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "fn multi_scope() {\n    {\n        let a = 1;\n    }\n    {\n        let b = 2;\n    }\n}\n");

    // Cursor on each `let` keyword: sibling blocks, shared fn body, fn item and file.
    let result = selection_ranges(&mut dsp, 602, &file, &[(2, 8), (5, 8)]);
    assert_eq!(
        result,
        json!(
            [chain(&[(2, 8, 2, 11), (1, 4, 3, 5), (0, 17, 7, 1), (0, 0, 7, 1), (0, 0, 8, 0)]), chain(&[(5, 8, 5, 11), (4, 4, 6, 5), (0, 17, 7, 1), (0, 0, 7, 1), (0, 0, 8, 0)]),]
        )
    );
}

#[test]
fn test_selection_range_batch_positions_ordered_descending() {
    let file = temp_file("desc");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "fn alpha() {\n    let x = 1;\n}\nfn omega() {\n    let y = 2;\n}\n");

    // Results follow request order, not document order.
    let result = selection_ranges(&mut dsp, 603, &file, &[(4, 8), (1, 8)]);
    assert_eq!(result, json!([chain(&[(4, 8, 4, 9), (3, 11, 5, 1), (3, 0, 5, 1), (0, 0, 6, 0)]), chain(&[(1, 8, 1, 9), (0, 11, 2, 1), (0, 0, 2, 1), (0, 0, 6, 0)]),]));
}

#[test]
fn test_selection_range_crlf_line_endings_position_calculation() {
    let file = temp_file("crlf");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "fn crlf_fn() {\r\n    let a = 100;\r\n}\r\n");

    // `\r` is not part of any range end: the body still ends right after `}`.
    let result = selection_ranges(&mut dsp, 604, &file, &[(1, 8)]);
    assert_eq!(result, json!([chain(&[(1, 8, 1, 9), (0, 13, 2, 1), (0, 0, 2, 1), (0, 0, 3, 0)])]));
}

#[test]
fn test_selection_range_utf8_multibyte_prefix_columns() {
    let file = temp_file("utf8");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "fn utf8_fn() {\n    // Γειάσα σας\n    let val = 42;\n}\n");

    // Greek letters are two UTF-8 bytes but one UTF-16 unit each: `val` below the comment
    // and the comment itself (13 UTF-16 units) both come back in UTF-16 columns.
    let result = selection_ranges(&mut dsp, 605, &file, &[(2, 8), (1, 9)]);
    assert_eq!(result, json!([chain(&[(2, 8, 2, 11), (0, 13, 3, 1), (0, 0, 3, 1), (0, 0, 4, 0)]), chain(&[(1, 4, 1, 17), (0, 13, 3, 1), (0, 0, 3, 1), (0, 0, 4, 0)]),]));
}

#[test]
fn test_selection_range_overlay_did_change_shifts_selection_range() {
    let file = temp_file("shift");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "fn shifted() {\n    let k = 1;\n}\n");

    // Prepend two comment lines; they attach to the fn item as leading trivia.
    let change_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": to_uri(&file), "version": 2 },
            "contentChanges": [{
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
                "text": "// comment 1\n// comment 2\n"
            }]
        }
    });
    assert!(dsp.dispatch(&change_msg));

    // `k` moved from line 1 to line 3; the body moved to line 2; the item starts at the comments.
    let result = selection_ranges(&mut dsp, 606, &file, &[(3, 8)]);
    assert_eq!(result, json!([chain(&[(3, 8, 3, 9), (2, 13, 4, 1), (0, 0, 4, 1), (0, 0, 5, 0)])]));
}
