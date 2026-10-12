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

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::path_to_uri;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition};
use serde_json::json;
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
        def_id: 1, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "test_fn".to_string(), path: format!("{}::test_fn", name),
        signature: None, file: "src/lib.rs".to_string(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 10,
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
    cancelled: Arc<Mutex<HashSet<serde_json::Value>>>,
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
fn test_selection_range_single_position_nested_scope() {
    let ts = create_temp_shard("single_pos");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn compute() {\n    let val = 42;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 201, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 1, "character": 8 }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 201);
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    let inner = &results[0];
    assert_eq!(inner["range"]["start"]["line"], 0);
    assert_eq!(inner["range"]["start"]["character"], 3);
    assert_eq!(inner["range"]["end"]["line"], 2);
    assert_eq!(inner["range"]["end"]["character"], 1);
    assert_eq!(inner["parent"]["range"]["start"]["line"], 0);
    assert_eq!(inner["parent"]["range"]["start"]["character"], 0);
    assert_eq!(inner["parent"]["range"]["end"]["line"], 3);
    assert_eq!(inner["parent"]["range"]["end"]["character"], 0);
}

#[test]
fn test_selection_range_cursor_inside_inner_block() {
    let ts = create_temp_shard("inner_block");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn outer() {\n    if true {\n        let inner_val = 100;\n    }\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 202, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 2, "character": 12 }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    let child = &results[0];
    let parent = &child["parent"];
    assert!(parent.is_object());
    let grandparent = &parent["parent"];
    assert!(grandparent.is_object());
}

#[test]
fn test_selection_range_multiple_positions_batch() {
    let ts = create_temp_shard("batch_pos");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn f1() {\n    let a = 1;\n}\nfn f2() {\n    let b = 2;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 203, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [
                { "line": 1, "character": 8 },
                { "line": 4, "character": 8 }
            ]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 2);
}

#[test]
fn test_selection_range_empty_positions_returns_empty() {
    let ts = create_temp_shard("empty_pos");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": { "uri": to_uri(&file), "languageId": "rust", "version": 1, "text": "fn simple() {}\n" }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 204, "method": "textDocument/selectionRange",
        "params": { "textDocument": { "uri": to_uri(&file) }, "positions": [] }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 0);

    // Positive control query on valid position
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 2040, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 0, "character": 4 }]
        }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_results = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_results.len(), 1);
    assert_eq!(ctrl_results[0]["range"]["start"]["line"], 0);
    assert_eq!(ctrl_results[0]["range"]["end"]["line"], 0);
}

#[test]
fn test_selection_range_missing_positions_key() {
    let ts = create_temp_shard("missing_pos");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": { "uri": to_uri(&file), "languageId": "rust", "version": 1, "text": "fn simple() {}\n" }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 205, "method": "textDocument/selectionRange",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 0);

    // Positive control query with explicit positions
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 2050, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 0, "character": 4 }]
        }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_results = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_results.len(), 1);
    assert_eq!(ctrl_results[0]["range"]["start"]["line"], 0);
    assert_eq!(ctrl_results[0]["range"]["end"]["line"], 0);
}

#[test]
fn test_selection_range_unopened_document_returns_empty() {
    let ts = create_temp_shard("unopened");
    let file = ts.dir.join("src/unopened.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 206, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 0, "character": 0 }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 0);

    // Positive control query by opening document
    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": { "uri": to_uri(&file), "languageId": "rust", "version": 1, "text": "fn opened() {}\n" }
        }
    });
    assert!(dsp.dispatch(&open_msg));
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 2060, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 0, "character": 0 }]
        }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_results = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_results.len(), 1);
    assert_eq!(ctrl_results[0]["range"]["start"]["line"], 0);
    assert_eq!(ctrl_results[0]["range"]["end"]["line"], 1);
}

#[test]
fn test_selection_range_cursor_at_start_of_file() {
    let ts = create_temp_shard("start_file");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": { "uri": to_uri(&file), "languageId": "rust", "version": 1, "text": "fn simple() {\n}\n" }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 207, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 0, "character": 0 }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["range"]["start"]["line"], 0);
    assert_eq!(results[0]["range"]["end"]["line"], 2);
}

#[test]
fn test_selection_range_cursor_at_eof() {
    let ts = create_temp_shard("eof_pos");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": { "uri": to_uri(&file), "languageId": "rust", "version": 1, "text": "fn simple() {\n    let x = 1;\n}\n" }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 208, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 3, "character": 0 }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["range"]["start"]["line"], 0);
    assert_eq!(results[0]["range"]["end"]["line"], 3);
}
