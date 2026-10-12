/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/semanticTokens/full/delta and range (Domain 7, Part 2).
//! Tests delta diffing, uncached fallback, range bounded queries, and request ID echoing.

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

static ST_P2_COUNTER: AtomicU64 = AtomicU64::new(1);

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
    let id = ST_P2_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_semtok_p2_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let mut writer = IndexWriter::new(name, "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "first".to_string(), path: format!("{}::first", name),
        signature: None, file: "src/lib.rs".to_string(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 6,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "second".to_string(), path: format!("{}::second", name),
        signature: None, file: "src/lib.rs".to_string(),
        start_line: 3, start_col: 1, end_line: 3, end_col: 7,
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
            msg.clone(), &mut self.writer, &mut self.lifecycle,
            &mut self.multi_root, &mut self.overlay_mgr, &self.cancelled, None,
        ).unwrap()
    }

    fn parse_response(&mut self) -> Value {
        let text = String::from_utf8(self.writer.clone()).unwrap();
        self.writer.clear();
        let body = text.split("\r\n\r\n").nth(1).unwrap();
        serde_json::from_str(body).unwrap()
    }
}

#[test]
fn test_semantic_tokens_delta_identical_data_returns_empty_edits() {
    let ts = create_temp_shard("delta_empty");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn first() {}\n\nfn second() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let full_req = json!({
        "jsonrpc": "2.0", "id": 791, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&full_req));
    let full_resp = dsp.parse_response();
    let res_id = full_resp["result"]["resultId"].as_str().unwrap();

    let delta_req = json!({
        "jsonrpc": "2.0", "id": 792, "method": "textDocument/semanticTokens/full/delta",
        "params": { "textDocument": { "uri": to_uri(&file) }, "previousResultId": res_id }
    });
    assert!(dsp.dispatch(&delta_req));
    let delta_resp = dsp.parse_response();
    assert_eq!(delta_resp["id"], 792);
    let edits = delta_resp["result"]["edits"].as_array().unwrap();
    assert_eq!(edits.len(), 0);
}

#[test]
fn test_semantic_tokens_delta_modified_data_returns_edit() {
    let ts = create_temp_shard("delta_mod");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn first() {}\n\nfn second() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": { "uri": to_uri(&file), "languageId": "rust", "version": 1, "text": "fn first() {}\n\nfn second() {}\n" }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let full_req = json!({
        "jsonrpc": "2.0", "id": 793, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&full_req));
    let full_resp = dsp.parse_response();
    let res_id = full_resp["result"]["resultId"].as_str().unwrap();

    let change_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": to_uri(&file), "version": 2 },
            "contentChanges": [{ "text": "fn modified_first() {}\n\nfn second() {}\n" }]
        }
    });
    assert!(dsp.dispatch(&change_msg));

    let delta_req = json!({
        "jsonrpc": "2.0", "id": 794, "method": "textDocument/semanticTokens/full/delta",
        "params": { "textDocument": { "uri": to_uri(&file) }, "previousResultId": res_id }
    });
    assert!(dsp.dispatch(&delta_req));
    let delta_resp = dsp.parse_response();
    assert_eq!(delta_resp["id"], 794);
    let edits = delta_resp["result"]["edits"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
}

#[test]
fn test_semantic_tokens_delta_uncached_id_returns_full_data() {
    let ts = create_temp_shard("delta_uncached");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn first() {}\n\nfn second() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let delta_req = json!({
        "jsonrpc": "2.0", "id": 795, "method": "textDocument/semanticTokens/full/delta",
        "params": { "textDocument": { "uri": to_uri(&file) }, "previousResultId": "unknown-nonexistent-id" }
    });
    assert!(dsp.dispatch(&delta_req));
    let delta_resp = dsp.parse_response();
    assert_eq!(delta_resp["id"], 795);
    let data = delta_resp["result"]["data"].as_array().unwrap();
    assert_eq!(data.len(), 10);
}

#[test]
fn test_semantic_tokens_range_bounded_first_function() {
    let ts = create_temp_shard("range_first");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn first() {}\n\nfn second() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 796, "method": "textDocument/semanticTokens/range",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 13 } }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 796);
    let data = resp["result"]["data"].as_array().unwrap();
    assert_eq!(data.len(), 5);
    assert_eq!(data[2], 5); // "first" length
}

#[test]
fn test_semantic_tokens_range_bounded_second_function() {
    let ts = create_temp_shard("range_second");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn first() {}\n\nfn second() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 797, "method": "textDocument/semanticTokens/range",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "range": { "start": { "line": 2, "character": 0 }, "end": { "line": 2, "character": 14 } }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 797);
    let data = resp["result"]["data"].as_array().unwrap();
    assert_eq!(data.len(), 5);
    assert_eq!(data[2], 6); // "second" length
}

#[test]
fn test_semantic_tokens_range_empty_window() {
    let ts = create_temp_shard("range_empty");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn first() {}\n\nfn second() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 798, "method": "textDocument/semanticTokens/range",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "range": { "start": { "line": 1, "character": 0 }, "end": { "line": 1, "character": 0 } }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let data = resp["result"]["data"].as_array().unwrap();
    assert_eq!(data.len(), 0);

    // Positive control query spanning both functions
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 7980, "method": "textDocument/semanticTokens/range",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 2, "character": 14 } }
        }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_data = ctrl_resp["result"]["data"].as_array().unwrap();
    assert_eq!(ctrl_data.len(), 10);
}

#[test]
fn test_semantic_tokens_range_beyond_eof() {
    let ts = create_temp_shard("range_eof");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn first() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 799, "method": "textDocument/semanticTokens/range",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "range": { "start": { "line": 100, "character": 0 }, "end": { "line": 200, "character": 0 } }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let data = resp["result"]["data"].as_array().unwrap();
    assert_eq!(data.len(), 0);

    // Positive control query covering valid line 0
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 7990, "method": "textDocument/semanticTokens/range",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 10 } }
        }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_data = ctrl_resp["result"]["data"].as_array().unwrap();
    assert_eq!(ctrl_data.len(), 5);
}

#[test]
fn test_semantic_tokens_range_echoes_numeric_id() {
    let ts = create_temp_shard("range_num_id");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn first() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 444333, "method": "textDocument/semanticTokens/range",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 10 } }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 444333);
    let data = resp["result"]["data"].as_array().unwrap();
    assert_eq!(data.len(), 5);
    assert_eq!(data[2], 5);
}

#[test]
fn test_semantic_tokens_range_echoes_string_id() {
    let ts = create_temp_shard("range_str_id");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn first() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": "st-range-str-42", "method": "textDocument/semanticTokens/range",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 10 } }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], "st-range-str-42");
    let data = resp["result"]["data"].as_array().unwrap();
    assert_eq!(data.len(), 5);
    assert_eq!(data[2], 5);
}

#[test]
fn test_semantic_tokens_delta_echoes_string_id() {
    let ts = create_temp_shard("delta_str_id");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn first() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": "st-delta-str-99", "method": "textDocument/semanticTokens/full/delta",
        "params": { "textDocument": { "uri": to_uri(&file) }, "previousResultId": "any-id" }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], "st-delta-str-99");
    let data = resp["result"]["data"].as_array().unwrap();
    assert_eq!(data.len(), 10);
    assert_eq!(data[2], 5);
}
