/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/selectionRange (Domain 7, Part 9).
//! Tests closed files on disk through the dispatcher, method-call boundaries, Unicode
//! identifiers next to emoji literals, and chain invariants over every cursor position.

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

use crate::ntm_domain7_selection_range_support::assert_chain_invariants;

static SEL_P9_COUNTER: AtomicU64 = AtomicU64::new(900);

fn to_uri(p: &Path) -> String {
    path_to_uri(p.to_str().unwrap())
}

fn temp_file(tag: &str) -> PathBuf {
    let id = SEL_P9_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("ridx_sel_p9_{}_{}_{}.rs", tag, std::process::id(), id))
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
fn test_selection_range_closed_file_on_disk_matches_open_document_through_dispatcher() {
    let file = temp_file("disk");
    let text = "fn disk() {\n    let d = 1;\n}\n";
    std::fs::write(&file, text).unwrap();
    let mut dsp = TestDispatcher::new();

    // Never opened: the server parses the file from disk.
    let expected = json!([chain(&[(1, 8, 1, 9), (0, 10, 2, 1), (0, 0, 2, 1), (0, 0, 3, 0)])]);
    assert_eq!(selection_ranges(&mut dsp, 901, &file, &[(1, 8)]), expected);

    // Opened with the same text: identical answer. Edited: the live buffer wins.
    open_doc(&mut dsp, &file, text);
    assert_eq!(selection_ranges(&mut dsp, 902, &file, &[(1, 8)]), expected);
    let change_msg = json!({ "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": { "textDocument": { "uri": to_uri(&file), "version": 2 },
            "contentChanges": [{ "text": "fn disk() {\n    let dd = 1;\n}\n" }] } });
    assert!(dsp.dispatch(&change_msg));
    assert_eq!(selection_ranges(&mut dsp, 903, &file, &[(1, 8)]), json!([chain(&[(1, 8, 1, 10), (0, 10, 2, 1), (0, 0, 2, 1), (0, 0, 3, 0)])]));

    // Closed again: back to the file on disk.
    let close_msg = json!({ "jsonrpc": "2.0", "method": "textDocument/didClose",
        "params": { "textDocument": { "uri": to_uri(&file) } } });
    assert!(dsp.dispatch(&close_msg));
    assert_eq!(selection_ranges(&mut dsp, 904, &file, &[(1, 8)]), expected);
    let _ = std::fs::remove_file(&file);
}

#[test]
fn test_selection_range_method_call_receiver_and_name_boundaries() {
    let file = temp_file("methodcall");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "fn call() {\n    foo.bar();\n}\n");

    // Between `foo` and `.` the receiver is selected; on `bar` and at its end before `(`
    // the method name is.
    let result = selection_ranges(&mut dsp, 905, &file, &[(1, 7), (1, 8), (1, 11)]);
    let foo = chain(&[(1, 4, 1, 7), (0, 10, 2, 1), (0, 0, 2, 1), (0, 0, 3, 0)]);
    let bar = chain(&[(1, 8, 1, 11), (0, 10, 2, 1), (0, 0, 2, 1), (0, 0, 3, 0)]);
    assert_eq!(result, json!([foo, bar, bar]));
}

#[test]
fn test_selection_range_unicode_identifier_after_emoji_literal_uses_utf16_columns() {
    let file = temp_file("unicode");
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, "fn ü() {\n    let s = \"🎉\"; let γειά = 1;\n}\n");

    // `🎉` is one char, four UTF-8 bytes and two UTF-16 units: `γειά` starts at UTF-16
    // column 22. The emoji string is a whole node; `ü` is a one-unit identifier.
    let result = selection_ranges(&mut dsp, 906, &file, &[(1, 23), (1, 14), (0, 3)]);
    assert_eq!(
        result,
        json!([
            chain(&[(1, 22, 1, 26), (0, 7, 2, 1), (0, 0, 2, 1), (0, 0, 3, 0)]),
            chain(&[(1, 12, 1, 16), (0, 7, 2, 1), (0, 0, 2, 1), (0, 0, 3, 0)]),
            chain(&[(0, 3, 0, 4), (0, 0, 2, 1), (0, 0, 3, 0)]),
        ])
    );
}

#[test]
fn test_selection_range_chain_invariants_hold_for_every_cursor_position() {
    let file = temp_file("invariants");
    let text = concat!(
        "/// Doc.\n",
        "#[inline]\n",
        "fn every(x: &'static str) -> usize {\n",
        "    // comment with words\n",
        "    let s = \"text { brace\"; let n = x.len();\n",
        "    if n > 0 {\n",
        "        { n }\n",
        "    } else {\n",
        "        0\n",
        "    }\n",
        "}\n",
    );
    let mut dsp = TestDispatcher::new();
    open_doc(&mut dsp, &file, text);

    let mut positions = Vec::new();
    for (line, content) in text.split('\n').enumerate() {
        for col in 0..=content.encode_utf16().count() as u32 {
            positions.push((line as u32, col));
        }
    }
    let result = selection_ranges(&mut dsp, 907, &file, &positions);
    let items = result.as_array().unwrap();
    assert_eq!(items.len(), positions.len());
    let last_line = text.matches('\n').count() as u64;
    for (item, &cursor) in items.iter().zip(&positions) {
        assert_chain_invariants(item, cursor, last_line);
    }
    // The `{` inside the string literal opens no block: on `brace` the string is innermost
    // and its parent is the fn body.
    let on_brace = positions.iter().position(|&p| p == (4, 20)).unwrap();
    assert_eq!(items[on_brace], chain(&[(4, 12, 4, 26), (2, 35, 10, 1), (0, 0, 10, 1), (0, 0, 11, 0)]));
}
