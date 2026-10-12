/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/semanticTokens/full (Domain 7, Part 1).
//! Tests relative 5-tuple delta encoding, overlay buffer updates, resultId formatting, and controls.

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

static ST_COUNTER: AtomicU64 = AtomicU64::new(1);

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
    let id = ST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_semtok_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let mut writer = IndexWriter::new(name, "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "test_fn".to_string(), path: format!("{}::test_fn", name),
        signature: None, file: "src/lib.rs".to_string(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 8,
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
fn test_semantic_tokens_full_single_function_disk() {
    let ts = create_temp_shard("single");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn test_fn() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 781, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 781);
    let data = resp["result"]["data"].as_array().unwrap();
    assert_eq!(data.len(), 5);
    assert_eq!(data[0], 0); // deltaLine
    assert_eq!(data[1], 0); // deltaStart
    assert_eq!(data[2], 7); // length
    assert_eq!(data[3], 11); // tokenType (function)
    assert_eq!(data[4], 2); // tokenModifiers (definition)
}

#[test]
fn test_semantic_tokens_full_multiple_functions_relative_delta_encoding() {
    let id = ST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_st_multi_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let mut writer = IndexWriter::new("st_multi", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "first".to_string(), path: "st_multi::first".to_string(),
        signature: None, file: "src/lib.rs".to_string(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 6,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "second".to_string(), path: "st_multi::second".to_string(),
        signature: None, file: "src/lib.rs".to_string(),
        start_line: 3, start_col: 1, end_line: 3, end_col: 7,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    let file = dir.join("src/lib.rs");
    std::fs::write(&file, "fn first() {}\n\nfn second() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 782, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let data = resp["result"]["data"].as_array().unwrap();
    assert_eq!(data.len(), 10);
    // Token 1
    assert_eq!(data[0], 0);
    assert_eq!(data[1], 0);
    assert_eq!(data[2], 5);
    // Token 2 (deltaLine = 2, deltaStart = 0)
    assert_eq!(data[5], 2);
    assert_eq!(data[6], 0);
    assert_eq!(data[7], 6);
}

#[test]
fn test_semantic_tokens_full_overlay_buffer_precedence() {
    let ts = create_temp_shard("overlay_prec");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn disk_fn() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn live() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 783, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let data = resp["result"]["data"].as_array().unwrap();
    assert_eq!(data.len(), 5);
    assert_eq!(data[0], 0);
    assert_eq!(data[1], 3);
    assert_eq!(data[2], 4); // "live" length
}

#[test]
fn test_semantic_tokens_full_overlay_did_change() {
    let ts = create_temp_shard("overlay_change");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn old() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let change_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": to_uri(&file), "version": 2 },
            "contentChanges": [{ "text": "fn updated_longer() {}\n" }]
        }
    });
    assert!(dsp.dispatch(&change_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 784, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let data = resp["result"]["data"].as_array().unwrap();
    assert_eq!(data.len(), 5);
    assert_eq!(data[2], 14); // "updated_longer" length
}

#[test]
fn test_semantic_tokens_full_overlay_did_close() {
    let ts = create_temp_shard("overlay_close");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn test_fn() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn temp() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let close_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didClose",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&close_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 785, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let data = resp["result"]["data"].as_array().unwrap();
    assert_eq!(data.len(), 5);
    assert_eq!(data[2], 7); // "test_fn" length
}

#[test]
fn test_semantic_tokens_full_empty_file_returns_empty_data() {
    let ts = create_temp_shard("empty_file");
    let file = ts.dir.join("src/empty.rs");
    std::fs::write(&file, "").unwrap();
    let valid_file = ts.dir.join("src/lib.rs");
    std::fs::write(&valid_file, "fn test_fn() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 786, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let data = resp["result"]["data"].as_array().unwrap();
    assert_eq!(data.len(), 0);

    // Positive control query on valid file
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 7860, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&valid_file) } }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_data = ctrl_resp["result"]["data"].as_array().unwrap();
    assert_eq!(ctrl_data.len(), 5);
}

#[test]
fn test_semantic_tokens_full_nonexistent_file_returns_empty_data() {
    let ts = create_temp_shard("missing");
    let file = ts.dir.join("src/missing.rs");
    let valid_file = ts.dir.join("src/lib.rs");
    std::fs::write(&valid_file, "fn test_fn() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 787, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let data = resp["result"]["data"].as_array().unwrap();
    assert_eq!(data.len(), 0);

    // Positive control query on valid file
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 7870, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&valid_file) } }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_data = ctrl_resp["result"]["data"].as_array().unwrap();
    assert_eq!(ctrl_data.len(), 5);
}

#[test]
fn test_semantic_tokens_full_result_id_format() {
    let ts = create_temp_shard("res_id");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn test_fn() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 788, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let result_id = resp["result"]["resultId"].as_str().unwrap();
    assert_eq!(&result_id[..8], "ridx-st-");
}

#[test]
fn test_semantic_tokens_full_dispatcher_echoes_numeric_id() {
    let ts = create_temp_shard("numeric_id");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn test_fn() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 888222, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 888222);
    let data = resp["result"]["data"].as_array().unwrap();
    assert_eq!(data.len(), 5);
    assert_eq!(data[2], 7);
}

#[test]
fn test_semantic_tokens_full_dispatcher_echoes_string_id() {
    let ts = create_temp_shard("string_id");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn test_fn() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": "st-full-str-id", "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], "st-full-str-id");
    let data = resp["result"]["data"].as_array().unwrap();
    assert_eq!(data.len(), 5);
    assert_eq!(data[2], 7);
}
