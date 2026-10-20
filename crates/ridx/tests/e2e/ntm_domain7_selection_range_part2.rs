/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/selectionRange (Domain 7, Part 2).
//! Tests deep nesting, live edits, ID echo, loops, invariants, and document isolation.

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::path_to_uri;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::ntm_domain7_selection_range_support::assert_chain_invariants;

static SEL_COUNTER2: AtomicU64 = AtomicU64::new(100);

struct TempShard {
    dir: PathBuf,
    shard: PathBuf,
}

impl Drop for TempShard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn create_temp_shard(name: &str) -> TempShard {
    let id = SEL_COUNTER2.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("ridx_sel2_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let mut writer = IndexWriter::new(name, "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "test_fn".to_string(),
        path: format!("{}::test_fn", name),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    TempShard { dir, shard }
}

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
    fn new(shard: PathBuf) -> Self {
        let mut d = Self {
            lifecycle: LifecycleManager::new(),
            multi_root: MultiRootWorkspace::new(),
            overlay_mgr: OverlayManager::new(),
            cancelled: Arc::new(Mutex::new(HashSet::new())),
            writer: Vec::new(),
        };
        assert!(d.multi_root.add_root(shard));
        let init_msg = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } });
        assert!(d.dispatch(&init_msg));
        d.writer.clear();
        d
    }

    fn dispatch(&mut self, msg: &Value) -> bool {
        dispatch_lsp_message(
            msg.clone(),
            &mut self.writer,
            &mut self.lifecycle,
            &mut self.multi_root,
            &mut self.overlay_mgr,
            &self.cancelled,
            None,
        )
        .unwrap()
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

fn selection_ranges(
    dsp: &mut TestDispatcher,
    id: Value,
    file: &Path,
    positions: &[(u32, u32)],
) -> Value {
    let positions: Vec<Value> = positions
        .iter()
        .map(|&(l, c)| json!({ "line": l, "character": c }))
        .collect();
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
fn test_selection_range_deeply_nested_blocks() {
    let ts = create_temp_shard("deep_nested");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    open_doc(
        &mut dsp, &file,
        "fn l1() {\n    if true {\n        while true {\n            let deepest = 1;\n        }\n    }\n}\n",
    );

    // Six levels: `deepest`, while block, if block, fn body, fn item, file.
    let result = selection_ranges(&mut dsp, json!(211), &file, &[(3, 16)]);
    assert_eq!(
        result,
        json!([chain(&[
            (3, 16, 3, 23),
            (2, 19, 4, 9),
            (1, 12, 5, 5),
            (0, 8, 6, 1),
            (0, 0, 6, 1),
            (0, 0, 7, 0)
        ])])
    );
}

#[test]
fn test_selection_range_after_did_change_overlay_edit() {
    let ts = create_temp_shard("did_change");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    open_doc(&mut dsp, &file, "fn compute() {\n    let val = 1;\n}\n");
    let change_msg = json!({ "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": { "textDocument": { "uri": to_uri(&file), "version": 2 },
            "contentChanges": [{ "text": "fn compute() {\n    if true {\n        let val = 1;\n    }\n}\n" }] } });
    assert!(dsp.dispatch(&change_msg));

    // The `if` block inserted by the edit is now between the token and the fn body.
    let result = selection_ranges(&mut dsp, json!(212), &file, &[(2, 12)]);
    assert_eq!(
        result,
        json!([chain(&[
            (2, 12, 2, 15),
            (1, 12, 3, 5),
            (0, 13, 4, 1),
            (0, 0, 4, 1),
            (0, 0, 5, 0)
        ])])
    );
}

#[test]
fn test_selection_range_numeric_request_id_echoed() {
    let ts = create_temp_shard("num_id");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    open_doc(&mut dsp, &file, "fn f() {}\n");

    let result = selection_ranges(&mut dsp, json!(501), &file, &[(0, 3)]);
    assert_eq!(
        result,
        json!([chain(&[(0, 3, 0, 4), (0, 0, 0, 9), (0, 0, 1, 0)])])
    );
}

#[test]
fn test_selection_range_string_request_id_echoed() {
    let ts = create_temp_shard("str_id");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    open_doc(&mut dsp, &file, "fn f() {}\n");

    let result = selection_ranges(&mut dsp, json!("sel-range-1"), &file, &[(0, 3)]);
    assert_eq!(
        result,
        json!([chain(&[(0, 3, 0, 4), (0, 0, 0, 9), (0, 0, 1, 0)])])
    );
}

#[test]
fn test_selection_range_cursor_on_closure_or_loop() {
    let ts = create_temp_shard("closure_loop");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    open_doc(
        &mut dsp,
        &file,
        "fn process() {\n    for x in 0..10 {\n        let y = x * 2;\n    }\n}\n",
    );

    let result = selection_ranges(&mut dsp, json!(213), &file, &[(2, 12)]);
    assert_eq!(
        result,
        json!([chain(&[
            (2, 12, 2, 13),
            (1, 19, 3, 5),
            (0, 13, 4, 1),
            (0, 0, 4, 1),
            (0, 0, 5, 0)
        ])])
    );
}

#[test]
fn test_selection_range_range_coordinates_validity() {
    let ts = create_temp_shard("coords_valid");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    open_doc(&mut dsp, &file, "fn valid() {\n    let a = 10;\n}\n");

    let result = selection_ranges(&mut dsp, json!(214), &file, &[(1, 8)]);
    assert_eq!(
        result,
        json!([chain(&[
            (1, 8, 1, 9),
            (0, 11, 2, 1),
            (0, 0, 2, 1),
            (0, 0, 3, 0)
        ])])
    );
    assert_chain_invariants(&result[0], (1, 8), 3);
}

#[test]
fn test_selection_range_parent_nesting_invariants() {
    let ts = create_temp_shard("nesting_inv");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    open_doc(
        &mut dsp,
        &file,
        "fn parent_test() {\n    let inner = 5;\n}\n",
    );

    // Every cursor position of the document yields a well-formed chain.
    let text = "fn parent_test() {\n    let inner = 5;\n}\n";
    let mut positions = Vec::new();
    for (line, content) in text.split('\n').enumerate() {
        for col in 0..=content.encode_utf16().count() as u32 {
            positions.push((line as u32, col));
        }
    }
    let result = selection_ranges(&mut dsp, json!(215), &file, &positions);
    let items = result.as_array().unwrap();
    assert_eq!(items.len(), positions.len());
    let last_line = text.matches('\n').count() as u64;
    for (item, &cursor) in items.iter().zip(&positions) {
        assert_chain_invariants(item, cursor, last_line);
    }
    // Spot check: cursor on `inner`.
    let on_inner = positions.iter().position(|&p| p == (1, 8)).unwrap();
    assert_eq!(
        items[on_inner],
        chain(&[(1, 8, 1, 13), (0, 17, 2, 1), (0, 0, 2, 1), (0, 0, 3, 0)])
    );
}

#[test]
fn test_selection_range_duplicate_positions_in_request() {
    let ts = create_temp_shard("dup_pos");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    open_doc(&mut dsp, &file, "fn f() {\n    let x = 1;\n}\n");

    let result = selection_ranges(&mut dsp, json!(216), &file, &[(1, 8), (1, 8)]);
    let expected = chain(&[(1, 8, 1, 9), (0, 7, 2, 1), (0, 0, 2, 1), (0, 0, 3, 0)]);
    assert_eq!(result, json!([expected, expected]));
}

#[test]
fn test_selection_range_position_beyond_file_length() {
    let ts = create_temp_shard("oob_pos");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    open_doc(&mut dsp, &file, "fn f() {}\n");

    // Clamped to the end of the text: only the file range contains it.
    let result = selection_ranges(&mut dsp, json!(217), &file, &[(999, 999)]);
    assert_eq!(result, json!([chain(&[(0, 0, 1, 0)])]));
}

#[test]
fn test_selection_range_multiple_documents_isolation() {
    let ts = create_temp_shard("doc_isolation");
    let file_a = ts.dir.join("src/a.rs");
    let file_b = ts.dir.join("src/b.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    open_doc(&mut dsp, &file_a, "fn fa() {\n    let a = 1;\n}\n");
    open_doc(&mut dsp, &file_b, "fn fb() {\n    let bb = 2;\n}\n");

    // Same cursor, different documents: the token lengths differ (`a` vs `bb`).
    let result_a = selection_ranges(&mut dsp, json!(218), &file_a, &[(1, 8)]);
    assert_eq!(
        result_a,
        json!([chain(&[
            (1, 8, 1, 9),
            (0, 8, 2, 1),
            (0, 0, 2, 1),
            (0, 0, 3, 0)
        ])])
    );
    let result_b = selection_ranges(&mut dsp, json!(219), &file_b, &[(1, 8)]);
    assert_eq!(
        result_b,
        json!([chain(&[
            (1, 8, 1, 10),
            (0, 8, 2, 1),
            (0, 0, 2, 1),
            (0, 0, 3, 0)
        ])])
    );
}
