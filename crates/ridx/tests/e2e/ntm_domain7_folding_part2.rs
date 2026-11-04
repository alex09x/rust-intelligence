/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/foldingRange (Domain 7, Part 2).
//! Tests edge cases, unclosed braces, overlays, live edits, and ID echo.

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

static FOLD_COUNTER2: AtomicU64 = AtomicU64::new(100);

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
    let id = FOLD_COUNTER2.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_fold2_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let mut writer = IndexWriter::new(name, "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "test_fn2".to_string(),
        path: format!("{}::test_fn2", name),
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
        dispatch_lsp_message(msg.clone(), &mut self.writer, &mut self.lifecycle, &mut self.multi_root, &mut self.overlay_mgr, &self.cancelled, None).unwrap()
    }

    fn parse_response(&mut self) -> serde_json::Value {
        let text = String::from_utf8(self.writer.clone()).unwrap();
        self.writer.clear();
        let body = text.split("\r\n\r\n").nth(1).unwrap();
        serde_json::from_str(body).unwrap()
    }
}

#[test]
fn test_folding_range_unclosed_braces_handled_safely() {
    let ts = create_temp_shard("unclosed");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn outer() {\n    if true {\n        let a = 1;\n    }\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({ "jsonrpc": "2.0", "id": 110, "method": "textDocument/foldingRange", "params": { "textDocument": { "uri": to_uri(&file) } } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let ranges = resp["result"].as_array().unwrap();
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0]["startLine"], 1);
    assert_eq!(ranges[0]["endLine"], 3);
}

#[test]
fn test_folding_range_unmatched_closing_braces_ignored() {
    let ts = create_temp_shard("unmatched_close");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "}\nfn valid() {\n    let x = 1;\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({ "jsonrpc": "2.0", "id": 111, "method": "textDocument/foldingRange", "params": { "textDocument": { "uri": to_uri(&file) } } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let ranges = resp["result"].as_array().unwrap();
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0]["startLine"], 1);
    assert_eq!(ranges[0]["endLine"], 3);
}

#[test]
fn test_folding_range_empty_file_returns_empty_array() {
    let ts = create_temp_shard("empty_file");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({ "jsonrpc": "2.0", "id": 112, "method": "textDocument/foldingRange", "params": { "textDocument": { "uri": to_uri(&file) } } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let ranges = resp["result"].as_array().unwrap();
    assert_eq!(ranges.len(), 0);

    // Positive control query on file with folding
    let ctrl_file = ts.dir.join("src/ctrl.rs");
    std::fs::write(&ctrl_file, "fn has_folds() {\n    let a = 1;\n}\n").unwrap();
    let ctrl_req = json!({ "jsonrpc": "2.0", "id": 1120, "method": "textDocument/foldingRange", "params": { "textDocument": { "uri": to_uri(&ctrl_file) } } });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_ranges = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_ranges.len(), 1);
    assert_eq!(ctrl_ranges[0]["startLine"], 0);
    assert_eq!(ctrl_ranges[0]["endLine"], 2);
}

#[test]
fn test_folding_range_nonexistent_file_returns_empty_array() {
    let ts = create_temp_shard("nonexistent");
    let file = ts.dir.join("src/missing.rs");
    let existing_file = ts.dir.join("src/lib.rs");
    std::fs::write(&existing_file, "fn exists() {\n    let ok = 1;\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({ "jsonrpc": "2.0", "id": 113, "method": "textDocument/foldingRange", "params": { "textDocument": { "uri": to_uri(&file) } } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let ranges = resp["result"].as_array().unwrap();
    assert_eq!(ranges.len(), 0);

    // Positive control query on existing file
    let ctrl_req = json!({ "jsonrpc": "2.0", "id": 1130, "method": "textDocument/foldingRange", "params": { "textDocument": { "uri": to_uri(&existing_file) } } });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_ranges = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_ranges.len(), 1);
    assert_eq!(ctrl_ranges[0]["startLine"], 0);
    assert_eq!(ctrl_ranges[0]["endLine"], 2);
}

#[test]
fn test_folding_range_overlay_scopes_take_precedence() {
    let ts = create_temp_shard("overlay_precedence");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn disk() {\n    let d = 1;\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn live() {\n    let live1 = 10;\n    let live2 = 20;\n    let live3 = 30;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({ "jsonrpc": "2.0", "id": 114, "method": "textDocument/foldingRange", "params": { "textDocument": { "uri": to_uri(&file) } } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let ranges = resp["result"].as_array().unwrap();
    // The open buffer wins over the file on disk: its function spans 0..4.
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0]["startLine"], 0);
    assert_eq!(ranges[0]["endLine"], 4);
}

#[test]
fn test_folding_range_overlay_single_line_scope_filtered() {
    let ts = create_temp_shard("overlay_single");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn one_liner() { let a = 1; }"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({ "jsonrpc": "2.0", "id": 115, "method": "textDocument/foldingRange", "params": { "textDocument": { "uri": to_uri(&file) } } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let ranges = resp["result"].as_array().unwrap();
    assert_eq!(ranges.len(), 0);

    // Positive control query by didChange to multi-line
    let change_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": to_uri(&file), "version": 2 },
            "contentChanges": [{ "text": "fn multi_liner() {\n    let a = 1;\n}\n" }]
        }
    });
    assert!(dsp.dispatch(&change_msg));
    let ctrl_req = json!({ "jsonrpc": "2.0", "id": 1150, "method": "textDocument/foldingRange", "params": { "textDocument": { "uri": to_uri(&file) } } });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_ranges = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_ranges.len(), 1);
    assert_eq!(ctrl_ranges[0]["startLine"], 0);
    assert_eq!(ctrl_ranges[0]["endLine"], 2);
}

#[test]
fn test_folding_range_overlay_dirty_edit_updates_folding() {
    let ts = create_temp_shard("overlay_edit");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn f1() {\n    let a = 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req1 = json!({ "jsonrpc": "2.0", "id": 116, "method": "textDocument/foldingRange", "params": { "textDocument": { "uri": to_uri(&file) } } });
    assert!(dsp.dispatch(&req1));
    let resp1 = dsp.parse_response();
    let ranges1 = resp1["result"].as_array().unwrap();
    assert_eq!(ranges1.len(), 1);

    let change_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": to_uri(&file), "version": 2 },
            "contentChanges": [{ "text": "fn f1() {\n    let a = 1;\n}\nfn f2() {\n    let b = 2;\n}\n" }]
        }
    });
    assert!(dsp.dispatch(&change_msg));

    let req2 = json!({ "jsonrpc": "2.0", "id": 117, "method": "textDocument/foldingRange", "params": { "textDocument": { "uri": to_uri(&file) } } });
    assert!(dsp.dispatch(&req2));
    let resp2 = dsp.parse_response();
    let ranges2 = resp2["result"].as_array().unwrap();
    // Both functions of the edited buffer fold; there is no whole-file fold.
    assert_eq!(ranges2.len(), 2);
    assert_eq!(ranges2[0]["startLine"], 0);
    assert_eq!(ranges2[0]["endLine"], 2);
    assert_eq!(ranges2[1]["startLine"], 3);
    assert_eq!(ranges2[1]["endLine"], 5);
}

#[test]
fn test_folding_range_dispatcher_echoes_numeric_id() {
    let ts = create_temp_shard("num_id");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn dummy() {\n    let x = 1;\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({ "jsonrpc": "2.0", "id": 8888, "method": "textDocument/foldingRange", "params": { "textDocument": { "uri": to_uri(&file) } } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 8888);
    let ranges = resp["result"].as_array().unwrap();
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0]["startLine"], 0);
    assert_eq!(ranges[0]["endLine"], 2);
}

#[test]
fn test_folding_range_dispatcher_echoes_string_id() {
    let ts = create_temp_shard("str_id");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn dummy() {\n    let x = 1;\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({ "jsonrpc": "2.0", "id": "fold-custom-id", "method": "textDocument/foldingRange", "params": { "textDocument": { "uri": to_uri(&file) } } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], "fold-custom-id");
    let ranges = resp["result"].as_array().unwrap();
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0]["startLine"], 0);
    assert_eq!(ranges[0]["endLine"], 2);
}

#[test]
fn test_folding_range_deeply_nested_five_levels() {
    let ts = create_temp_shard("nested_5");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn l1() {\n    if true {\n        while true {\n            loop {\n                match 1 {\n                    _ => {}\n                }\n            }\n        }\n    }\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({ "jsonrpc": "2.0", "id": 119, "method": "textDocument/foldingRange", "params": { "textDocument": { "uri": to_uri(&file) } } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let ranges = resp["result"].as_array().unwrap();
    assert_eq!(ranges.len(), 5);
}
