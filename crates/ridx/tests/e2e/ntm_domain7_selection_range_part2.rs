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
use serde_json::json;
use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

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
    let dir = std::env::temp_dir().join(format!("ridx_sel2_{}_{}_{}", name, std::process::id(), id));
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
fn test_selection_range_deeply_nested_blocks() {
    let ts = create_temp_shard("deep_nested");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": { "uri": to_uri(&file), "languageId": "rust", "version": 1,
            "text": "fn l1() {\n    if true {\n        while true {\n            let deepest = 1;\n        }\n    }\n}\n" } }
    });
    assert!(dsp.dispatch(&open_msg));
    let req = json!({ "jsonrpc": "2.0", "id": 211, "method": "textDocument/selectionRange",
        "params": { "textDocument": { "uri": to_uri(&file) }, "positions": [{ "line": 3, "character": 16 }] } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    let mut depth = 0;
    let mut curr = Some(&results[0]);
    while let Some(r) = curr {
        depth += 1;
        curr = r["parent"].as_object().map(|_| &r["parent"]);
    }
    assert_eq!(depth, 4);
}

#[test]
fn test_selection_range_after_did_change_overlay_edit() {
    let ts = create_temp_shard("did_change");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let open_msg = json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": { "uri": to_uri(&file), "languageId": "rust", "version": 1, "text": "fn compute() {\n    let val = 1;\n}\n" } } });
    assert!(dsp.dispatch(&open_msg));
    let change_msg = json!({ "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": { "textDocument": { "uri": to_uri(&file), "version": 2 },
            "contentChanges": [{ "text": "fn compute() {\n    if true {\n        let val = 1;\n    }\n}\n" }] } });
    assert!(dsp.dispatch(&change_msg));
    let req = json!({ "jsonrpc": "2.0", "id": 212, "method": "textDocument/selectionRange",
        "params": { "textDocument": { "uri": to_uri(&file) }, "positions": [{ "line": 2, "character": 12 }] } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["range"]["start"]["line"], 1);
    assert_eq!(results[0]["parent"]["range"]["start"]["line"], 0);
}

#[test]
fn test_selection_range_numeric_request_id_echoed() {
    let ts = create_temp_shard("num_id");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let open_msg = json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": { "uri": to_uri(&file), "languageId": "rust", "version": 1, "text": "fn f() {}\n" } } });
    assert!(dsp.dispatch(&open_msg));
    let req = json!({ "jsonrpc": "2.0", "id": 501, "method": "textDocument/selectionRange",
        "params": { "textDocument": { "uri": to_uri(&file) }, "positions": [{ "line": 0, "character": 3 }] } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 501);
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["range"]["start"]["line"], 0);
    assert_eq!(results[0]["range"]["end"]["line"], 0);
}

#[test]
fn test_selection_range_string_request_id_echoed() {
    let ts = create_temp_shard("str_id");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let open_msg = json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": { "uri": to_uri(&file), "languageId": "rust", "version": 1, "text": "fn f() {}\n" } } });
    assert!(dsp.dispatch(&open_msg));
    let req = json!({ "jsonrpc": "2.0", "id": "sel-range-1", "method": "textDocument/selectionRange",
        "params": { "textDocument": { "uri": to_uri(&file) }, "positions": [{ "line": 0, "character": 3 }] } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], "sel-range-1");
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["range"]["start"]["line"], 0);
    assert_eq!(results[0]["range"]["end"]["line"], 0);
}

#[test]
fn test_selection_range_cursor_on_closure_or_loop() {
    let ts = create_temp_shard("closure_loop");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let open_msg = json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": { "uri": to_uri(&file), "languageId": "rust", "version": 1,
            "text": "fn process() {\n    for x in 0..10 {\n        let y = x * 2;\n    }\n}\n" } } });
    assert!(dsp.dispatch(&open_msg));
    let req = json!({ "jsonrpc": "2.0", "id": 213, "method": "textDocument/selectionRange",
        "params": { "textDocument": { "uri": to_uri(&file) }, "positions": [{ "line": 2, "character": 12 }] } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0]["parent"].is_object());
}

#[test]
fn test_selection_range_range_coordinates_validity() {
    let ts = create_temp_shard("coords_valid");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let open_msg = json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": { "uri": to_uri(&file), "languageId": "rust", "version": 1, "text": "fn valid() {\n    let a = 10;\n}\n" } } });
    assert!(dsp.dispatch(&open_msg));
    let req = json!({ "jsonrpc": "2.0", "id": 214, "method": "textDocument/selectionRange",
        "params": { "textDocument": { "uri": to_uri(&file) }, "positions": [{ "line": 1, "character": 8 }] } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    let sl = results[0]["range"]["start"]["line"].as_u64().unwrap();
    let el = results[0]["range"]["end"]["line"].as_u64().unwrap();
    assert_eq!(sl, 0);
    assert_eq!(el, 2);
}

#[test]
fn test_selection_range_parent_nesting_invariants() {
    let ts = create_temp_shard("nesting_inv");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let open_msg = json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": { "uri": to_uri(&file), "languageId": "rust", "version": 1, "text": "fn parent_test() {\n    let inner = 5;\n}\n" } } });
    assert!(dsp.dispatch(&open_msg));
    let req = json!({ "jsonrpc": "2.0", "id": 215, "method": "textDocument/selectionRange",
        "params": { "textDocument": { "uri": to_uri(&file) }, "positions": [{ "line": 1, "character": 8 }] } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let results = resp["result"].as_array().unwrap();
    let child = &results[0];
    let parent = &child["parent"];
    let c_sl = child["range"]["start"]["line"].as_u64().unwrap();
    let p_sl = parent["range"]["start"]["line"].as_u64().unwrap();
    assert_eq!(c_sl, 0);
    assert_eq!(p_sl, 0);
}

#[test]
fn test_selection_range_duplicate_positions_in_request() {
    let ts = create_temp_shard("dup_pos");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let open_msg = json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": { "uri": to_uri(&file), "languageId": "rust", "version": 1, "text": "fn f() {\n    let x = 1;\n}\n" } } });
    assert!(dsp.dispatch(&open_msg));
    let req = json!({ "jsonrpc": "2.0", "id": 216, "method": "textDocument/selectionRange",
        "params": { "textDocument": { "uri": to_uri(&file) }, "positions": [{ "line": 1, "character": 8 }, { "line": 1, "character": 8 }] } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0], results[1]);
}

#[test]
fn test_selection_range_position_beyond_file_length() {
    let ts = create_temp_shard("oob_pos");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let open_msg = json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": { "uri": to_uri(&file), "languageId": "rust", "version": 1, "text": "fn f() {}\n" } } });
    assert!(dsp.dispatch(&open_msg));
    let req = json!({ "jsonrpc": "2.0", "id": 217, "method": "textDocument/selectionRange",
        "params": { "textDocument": { "uri": to_uri(&file) }, "positions": [{ "line": 999, "character": 999 }] } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);
}

#[test]
fn test_selection_range_multiple_documents_isolation() {
    let ts = create_temp_shard("doc_isolation");
    let file_a = ts.dir.join("src/a.rs");
    let file_b = ts.dir.join("src/b.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let open_a = json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": { "uri": to_uri(&file_a), "languageId": "rust", "version": 1, "text": "fn fa() {\n    let a = 1;\n}\n" } } });
    assert!(dsp.dispatch(&open_a));
    let open_b = json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": { "uri": to_uri(&file_b), "languageId": "rust", "version": 1, "text": "fn fb() {\n    let b = 2;\n}\n" } } });
    assert!(dsp.dispatch(&open_b));
    let req_a = json!({ "jsonrpc": "2.0", "id": 218, "method": "textDocument/selectionRange",
        "params": { "textDocument": { "uri": to_uri(&file_a) }, "positions": [{ "line": 1, "character": 8 }] } });
    assert!(dsp.dispatch(&req_a));
    let resp_a = dsp.parse_response();
    let results_a = resp_a["result"].as_array().unwrap();
    assert_eq!(results_a.len(), 1);

    let req_b = json!({ "jsonrpc": "2.0", "id": 219, "method": "textDocument/selectionRange",
        "params": { "textDocument": { "uri": to_uri(&file_b) }, "positions": [{ "line": 1, "character": 8 }] } });
    assert!(dsp.dispatch(&req_b));
    let resp_b = dsp.parse_response();
    let results_b = resp_b["result"].as_array().unwrap();
    assert_eq!(results_b.len(), 1);
}

#[test]
fn test_selection_range_cursor_between_functions() {
    let ts = create_temp_shard("between_fns");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn f1() {\n    let a = 1;\n}\n\nfn f2() {\n    let b = 2;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 209, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 3, "character": 0 }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["parent"], serde_json::Value::Null);
}

#[test]
fn test_selection_range_nested_struct_impl_method() {
    let ts = create_temp_shard("struct_impl");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "struct Container {\n    val: i32,\n}\nimpl Container {\n    fn read(&self) -> i32 {\n        self.val\n    }\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 210, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 5, "character": 12 }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["range"]["start"]["line"], 4);
    assert_eq!(results[0]["parent"]["range"]["start"]["line"], 3);
}

