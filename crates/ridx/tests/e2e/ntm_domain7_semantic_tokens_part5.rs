/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/semanticTokens/full and delta (Domain 7, Part 5).
//! Tests UTF-16 encoding, overlay live symbols, dirty shift, deletion, and delta append.

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

static ST_P5_COUNTER: AtomicU64 = AtomicU64::new(1);

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
fn test_semantic_tokens_full_utf16_multibyte_token_coordinates() {
    let id = ST_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_semtok_p5_utf16_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("utf16_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "test_fn".to_string(), path: "utf16_crate::test_fn".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 10, end_line: 1, end_col: 17,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "// Γειάσα fn test_fn() {}\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 112, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 112);
    assert_eq!(resp["result"]["data"], json!([0, 9, 7, 11, 2]));
}

#[test]
fn test_semantic_tokens_full_overlay_live_symbols_precedence() {
    let id = ST_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_semtok_p5_live_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("live_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "disk_fn".to_string(), path: "live_crate::disk_fn".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 4, end_line: 1, end_col: 11,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn disk_fn() {}\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let did_open = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn live_fn() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&did_open));

    let req = json!({
        "jsonrpc": "2.0", "id": 113, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 113);
    let data = resp["result"]["data"].as_array().unwrap();
    assert_eq!(data[3], 11);
    assert_eq!(data[4], 2);
}

#[test]
fn test_semantic_tokens_full_overlay_dirty_line_shift() {
    let id = ST_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_semtok_p5_shift_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("shift_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "shifted".to_string(), path: "shift_crate::shifted".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 4, end_line: 1, end_col: 11,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn shifted() {}\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let did_open = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn shifted() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&did_open));

    let did_change = json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": to_uri(&file), "version": 2 },
            "contentChanges": [
                {
                    "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
                    "text": "\n\n"
                }
            ]
        }
    });
    assert!(dsp.dispatch(&did_change));

    let req = json!({
        "jsonrpc": "2.0", "id": 114, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 114);
    let data = resp["result"]["data"].as_array().unwrap();
    assert_eq!(data[0], 2);
    assert_eq!(data[2], 7);
}

#[test]
fn test_semantic_tokens_full_overlay_dirty_deletion_removes_token() {
    let id = ST_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_semtok_p5_del_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("del_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "deleted_fn".to_string(), path: "del_crate::deleted_fn".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 4, end_line: 1, end_col: 14,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn deleted_fn() {}\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let did_open = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn deleted_fn() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&did_open));

    let did_change = json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": to_uri(&file), "version": 2 },
            "contentChanges": [
                {
                    "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 1, "character": 0 } },
                    "text": ""
                }
            ]
        }
    });
    assert!(dsp.dispatch(&did_change));

    let req = json!({
        "jsonrpc": "2.0", "id": 115, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 115);
    assert_eq!(resp["result"]["data"], json!([]));
}

#[test]
fn test_semantic_tokens_delta_single_token_appended() {
    let id = ST_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_semtok_p5_delta_app_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("app_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "first".to_string(), path: "app_crate::first".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 4, end_line: 1, end_col: 9,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn first() {}\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let full_req = json!({
        "jsonrpc": "2.0", "id": 116, "method": "textDocument/semanticTokens/full",
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
                "text": "fn first() {}\nfn second() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&did_open));

    let delta_req = json!({
        "jsonrpc": "2.0", "id": 117, "method": "textDocument/semanticTokens/full/delta",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "previousResultId": prev_id
        }
    });
    assert!(dsp.dispatch(&delta_req));
    let delta_resp = dsp.parse_response();
    assert_eq!(delta_resp["id"], 117);
    let edits = delta_resp["result"]["edits"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0]["start"], 5);
    assert_eq!(edits[0]["deleteCount"], 0);
    assert_eq!(edits[0]["data"], json!([1, 3, 6, 11, 2]));
}
