/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/semanticTokens delta and range (Domain 7, Part 7).
//! Tests whitespace delta stability, single line slice, line range subsets, boundaries, and inverted ranges.

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

static ST_P7_COUNTER: AtomicU64 = AtomicU64::new(1);

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
        dispatch_lsp_message(msg.clone(), &mut self.writer, &mut self.lifecycle, &mut self.multi_root, &mut self.overlay_mgr, &self.cancelled, None).unwrap()
    }
    fn parse_response(&mut self) -> Value {
        let text = String::from_utf8(self.writer.clone()).unwrap();
        self.writer.clear();
        let body = text.split("\r\n\r\n").nth(1).unwrap();
        serde_json::from_str(body).unwrap()
    }
}

#[test]
fn test_semantic_tokens_delta_whitespace_did_change_returns_empty_edits() {
    let id = ST_P7_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_semtok_p7_ws_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("ws_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "bar".to_string(),
        path: "ws_crate::bar".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 4,
        end_line: 1,
        end_col: 7,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn bar() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);
    let full_req = json!({
        "jsonrpc": "2.0", "id": 127, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&full_req));
    let full_resp = dsp.parse_response();
    let prev_id = full_resp["result"]["resultId"].as_str().unwrap();
    let did_open = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn bar() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&did_open));
    let delta_req = json!({
        "jsonrpc": "2.0", "id": 128, "method": "textDocument/semanticTokens/full/delta",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "previousResultId": prev_id
        }
    });
    assert!(dsp.dispatch(&delta_req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 128);
    assert_eq!(resp["result"]["edits"], json!([]));
}

#[test]
fn test_semantic_tokens_range_single_line_slice() {
    let id = ST_P7_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_semtok_p7_rslice_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("rslice_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "f1".to_string(),
        path: "rslice_crate::f1".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 4,
        end_line: 1,
        end_col: 6,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "f2".to_string(),
        path: "rslice_crate::f2".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 3,
        start_col: 4,
        end_line: 3,
        end_col: 6,
    });
    writer.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "f3".to_string(),
        path: "rslice_crate::f3".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 5,
        start_col: 4,
        end_line: 5,
        end_col: 6,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn f1() {}\n\nfn f2() {}\n\nfn f3() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 129, "method": "textDocument/semanticTokens/range",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "range": { "start": { "line": 2, "character": 0 }, "end": { "line": 2, "character": 20 } }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 129);
    assert_eq!(resp["result"]["data"], json!([2, 3, 2, 11, 2]));
}

#[test]
fn test_semantic_tokens_range_spanning_subset_of_lines() {
    let id = ST_P7_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_semtok_p7_subset_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("subset_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "f1".to_string(),
        path: "subset_crate::f1".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 4,
        end_line: 1,
        end_col: 6,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "f2".to_string(),
        path: "subset_crate::f2".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 2,
        start_col: 4,
        end_line: 2,
        end_col: 6,
    });
    writer.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "f3".to_string(),
        path: "subset_crate::f3".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 3,
        start_col: 4,
        end_line: 3,
        end_col: 6,
    });
    writer.add_definition(RawDefinition {
        def_id: 4,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "f4".to_string(),
        path: "subset_crate::f4".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 4,
        start_col: 4,
        end_line: 4,
        end_col: 6,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn f1() {}\nfn f2() {}\nfn f3() {}\nfn f4() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 130, "method": "textDocument/semanticTokens/range",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "range": { "start": { "line": 1, "character": 0 }, "end": { "line": 2, "character": 20 } }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 130);
    assert_eq!(resp["result"]["data"], json!([1, 3, 2, 11, 2, 1, 3, 2, 11, 2]));
}

#[test]
fn test_semantic_tokens_range_boundary_start_line_exact() {
    let id = ST_P7_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_semtok_p7_bstart_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("bstart_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "foo".to_string(),
        path: "bstart_crate::foo".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 2,
        start_col: 4,
        end_line: 2,
        end_col: 7,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "\nfn foo() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 131, "method": "textDocument/semanticTokens/range",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "range": { "start": { "line": 1, "character": 3 }, "end": { "line": 5, "character": 0 } }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 131);
    assert_eq!(resp["result"]["data"], json!([1, 3, 3, 11, 2]));
}

#[test]
fn test_semantic_tokens_range_boundary_end_line_exact() {
    let id = ST_P7_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_semtok_p7_bend_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("bend_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "foo".to_string(),
        path: "bend_crate::foo".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 2,
        start_col: 4,
        end_line: 2,
        end_col: 7,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "\nfn foo() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 132, "method": "textDocument/semanticTokens/range",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 1, "character": 20 } }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 132);
    assert_eq!(resp["result"]["data"], json!([1, 3, 3, 11, 2]));
}

#[test]
fn test_semantic_tokens_range_inverted_lines_returns_empty() {
    let id = ST_P7_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_semtok_p7_inv_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("inv_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "foo".to_string(),
        path: "inv_crate::foo".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 4,
        end_line: 1,
        end_col: 7,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn foo() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 133, "method": "textDocument/semanticTokens/range",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "range": { "start": { "line": 10, "character": 0 }, "end": { "line": 2, "character": 20 } }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 133);
    assert_eq!(resp["result"]["data"], json!([]));
}
