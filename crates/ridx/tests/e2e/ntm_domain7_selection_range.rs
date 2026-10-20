/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/selectionRange (Domain 7, Part 1).
//! Tests scope hierarchy nesting, parent chains, batch positions, and boundary queries.
//!
//! A chain is innermost first: token under the cursor (if any), enclosing blocks, the `fn`
//! body `{..}`, the `fn` item (from its first attribute or doc comment), then the whole file.

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

static SEL_COUNTER: AtomicU64 = AtomicU64::new(1);

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
    let id = SEL_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_sel_{}_{}_{}", name, std::process::id(), id));
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
    id: u64,
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
fn test_selection_range_single_position_nested_scope() {
    let ts = create_temp_shard("single_pos");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    open_doc(&mut dsp, &file, "fn compute() {\n    let val = 42;\n}\n");

    // Cursor on `val`: token, fn body, fn item, file.
    let result = selection_ranges(&mut dsp, 201, &file, &[(1, 8)]);
    assert_eq!(
        result,
        json!([chain(&[
            (1, 8, 1, 11),
            (0, 13, 2, 1),
            (0, 0, 2, 1),
            (0, 0, 3, 0)
        ])])
    );
}

#[test]
fn test_selection_range_cursor_inside_inner_block() {
    let ts = create_temp_shard("inner_block");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    open_doc(
        &mut dsp,
        &file,
        "fn outer() {\n    if true {\n        let inner_val = 100;\n    }\n}\n",
    );

    // `inner_val` token, `if` block, fn body, fn item, file.
    let result = selection_ranges(&mut dsp, 202, &file, &[(2, 12)]);
    assert_eq!(
        result,
        json!([chain(&[
            (2, 12, 2, 21),
            (1, 12, 3, 5),
            (0, 11, 4, 1),
            (0, 0, 4, 1),
            (0, 0, 5, 0)
        ])])
    );
}

#[test]
fn test_selection_range_multiple_positions_batch() {
    let ts = create_temp_shard("batch_pos");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    open_doc(
        &mut dsp,
        &file,
        "fn f1() {\n    let a = 1;\n}\nfn f2() {\n    let b = 2;\n}\n",
    );

    let result = selection_ranges(&mut dsp, 203, &file, &[(1, 8), (4, 8)]);
    assert_eq!(
        result,
        json!([
            chain(&[(1, 8, 1, 9), (0, 8, 2, 1), (0, 0, 2, 1), (0, 0, 6, 0)]),
            chain(&[(4, 8, 4, 9), (3, 8, 5, 1), (3, 0, 5, 1), (0, 0, 6, 0)]),
        ])
    );
}

#[test]
fn test_selection_range_empty_positions_returns_empty() {
    let ts = create_temp_shard("empty_pos");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    open_doc(&mut dsp, &file, "fn simple() {}\n");

    assert_eq!(selection_ranges(&mut dsp, 204, &file, &[]), json!([]));

    // Positive control: cursor on `simple` sits in the fn item but before its body.
    let ctrl = selection_ranges(&mut dsp, 2040, &file, &[(0, 4)]);
    assert_eq!(
        ctrl,
        json!([chain(&[(0, 3, 0, 9), (0, 0, 0, 14), (0, 0, 1, 0)])])
    );
}

#[test]
fn test_selection_range_missing_positions_key() {
    let ts = create_temp_shard("missing_pos");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    open_doc(&mut dsp, &file, "fn simple() {}\n");

    let req = json!({
        "jsonrpc": "2.0", "id": 205, "method": "textDocument/selectionRange",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 205);
    assert_eq!(resp["result"], json!([]));

    let ctrl = selection_ranges(&mut dsp, 2050, &file, &[(0, 4)]);
    assert_eq!(
        ctrl,
        json!([chain(&[(0, 3, 0, 9), (0, 0, 0, 14), (0, 0, 1, 0)])])
    );
}

#[test]
fn test_selection_range_unopened_document_returns_empty() {
    let ts = create_temp_shard("unopened");
    let file = ts.dir.join("src/unopened.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    // Neither open nor on disk: nothing to select in.
    assert_eq!(selection_ranges(&mut dsp, 206, &file, &[(0, 0)]), json!([]));

    // Positive control after didOpen: the `fn` keyword is the innermost token.
    open_doc(&mut dsp, &file, "fn opened() {}\n");
    let ctrl = selection_ranges(&mut dsp, 2060, &file, &[(0, 0)]);
    assert_eq!(
        ctrl,
        json!([chain(&[(0, 0, 0, 2), (0, 0, 0, 14), (0, 0, 1, 0)])])
    );
}

#[test]
fn test_selection_range_cursor_at_start_of_file() {
    let ts = create_temp_shard("start_file");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    open_doc(&mut dsp, &file, "fn simple() {\n}\n");

    let result = selection_ranges(&mut dsp, 207, &file, &[(0, 0)]);
    assert_eq!(
        result,
        json!([chain(&[(0, 0, 0, 2), (0, 0, 1, 1), (0, 0, 2, 0)])])
    );
}

#[test]
fn test_selection_range_cursor_at_eof() {
    let ts = create_temp_shard("eof_pos");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    open_doc(&mut dsp, &file, "fn simple() {\n    let x = 1;\n}\n");

    // The empty line after the final newline is outside every item: file range only.
    let result = selection_ranges(&mut dsp, 208, &file, &[(3, 0)]);
    assert_eq!(result, json!([chain(&[(0, 0, 3, 0)])]));
}
