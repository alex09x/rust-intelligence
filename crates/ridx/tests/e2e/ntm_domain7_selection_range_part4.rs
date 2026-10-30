/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/selectionRange (Domain 7, Part 4).
//! Tests module items, traits, impls, closures, const blocks, and infinite loops.

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

static SEL_P4_COUNTER: AtomicU64 = AtomicU64::new(400);

fn to_uri(p: &Path) -> String {
    path_to_uri(p.to_str().unwrap())
}

fn temp_file(tag: &str) -> PathBuf {
    let id = SEL_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("ridx_sel_p4_{}_{}_{}.rs", tag, std::process::id(), id))
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
fn test_selection_range_cursor_inside_nested_module() {
    let file = temp_file("mod");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "mod outer {\n    mod inner {\n        fn nested_fn() {\n            let k = 1;\n        }\n    }\n}\n");

    // `k`, fn body, fn item (from its indented `fn`), inner mod block, outer mod block, file.
    let result = selection_ranges(&mut dsp, 401, &file, &[(3, 16)]);
    assert_eq!(result, json!([chain(&[(3, 16, 3, 17), (2, 23, 4, 9), (2, 8, 4, 9), (1, 14, 5, 5), (0, 10, 6, 1), (0, 0, 7, 0)])]));
}

#[test]
fn test_selection_range_cursor_inside_trait_default_method() {
    let file = temp_file("trait");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "trait Greeter {\n    fn hello(&self) {\n        let msg = \"hi\";\n    }\n}\n");

    let result = selection_ranges(&mut dsp, 402, &file, &[(2, 12)]);
    assert_eq!(result, json!([chain(&[(2, 12, 2, 15), (1, 20, 3, 5), (1, 4, 3, 5), (0, 14, 4, 1), (0, 0, 5, 0)])]));
}

#[test]
fn test_selection_range_cursor_inside_impl_block_method() {
    let file = temp_file("impl");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "struct Service;\nimpl Service {\n    fn run(&self) {\n        let active = true;\n    }\n}\n");

    let result = selection_ranges(&mut dsp, 403, &file, &[(3, 12)]);
    assert_eq!(result, json!([chain(&[(3, 12, 3, 18), (2, 18, 4, 5), (2, 4, 4, 5), (1, 13, 5, 1), (0, 0, 6, 0)])]));
}

#[test]
fn test_selection_range_cursor_inside_closure_block() {
    let file = temp_file("closure");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "fn with_closure() {\n    let cb = || {\n        compute();\n    };\n}\n");

    let result = selection_ranges(&mut dsp, 404, &file, &[(2, 10)]);
    assert_eq!(result, json!([chain(&[(2, 8, 2, 15), (1, 16, 3, 5), (0, 18, 4, 1), (0, 0, 4, 1), (0, 0, 5, 0)])]));
}

#[test]
fn test_selection_range_cursor_inside_const_item_block() {
    let file = temp_file("const");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "fn with_const() {\n    const LIMIT: i32 = {\n        100\n    };\n}\n");

    // A number is not a token: the chain starts at the const initializer block.
    let result = selection_ranges(&mut dsp, 405, &file, &[(2, 10)]);
    assert_eq!(result, json!([chain(&[(1, 23, 3, 5), (0, 16, 4, 1), (0, 0, 4, 1), (0, 0, 5, 0)])]));
}

#[test]
fn test_selection_range_cursor_inside_loop_infinite() {
    let file = temp_file("loop");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "fn infinite() {\n    loop {\n        tick();\n    }\n}\n");

    let result = selection_ranges(&mut dsp, 406, &file, &[(2, 10)]);
    assert_eq!(result, json!([chain(&[(2, 8, 2, 12), (1, 9, 3, 5), (0, 14, 4, 1), (0, 0, 4, 1), (0, 0, 5, 0)])]));
}
