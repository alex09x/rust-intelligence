/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/foldingRange (Domain 7, Part 1).
//! Tests disk fallback brace matching, comments, strings, char literals, and nesting.

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

static FOLD_COUNTER: AtomicU64 = AtomicU64::new(1);

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
    let id = FOLD_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_fold_{}_{}_{}", name, std::process::id(), id));
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
fn test_folding_range_single_function_disk_fallback() {
    let ts = create_temp_shard("single_fn");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn simple() {\n    let a = 1;\n    let b = 2;\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({ "jsonrpc": "2.0", "id": 101, "method": "textDocument/foldingRange", "params": { "textDocument": { "uri": to_uri(&file) } } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 101);
    let ranges = resp["result"].as_array().unwrap();
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0]["startLine"], 0);
    assert_eq!(ranges[0]["endLine"], 3);
    assert_eq!(ranges[0]["kind"], "region");
}

#[test]
fn test_folding_range_nested_blocks_disk() {
    let ts = create_temp_shard("nested");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn outer() {\n    if true {\n        let x = 10;\n    }\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({ "jsonrpc": "2.0", "id": 102, "method": "textDocument/foldingRange", "params": { "textDocument": { "uri": to_uri(&file) } } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let ranges = resp["result"].as_array().unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0]["startLine"], 1);
    assert_eq!(ranges[0]["endLine"], 3);
    assert_eq!(ranges[1]["startLine"], 0);
    assert_eq!(ranges[1]["endLine"], 4);
}

#[test]
fn test_folding_range_single_line_braces_ignored() {
    let ts = create_temp_shard("single_line");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn one_liner() { let x = 1; }\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({ "jsonrpc": "2.0", "id": 103, "method": "textDocument/foldingRange", "params": { "textDocument": { "uri": to_uri(&file) } } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let ranges = resp["result"].as_array().unwrap();
    assert_eq!(ranges.len(), 0);

    // Positive control query on multi-line block
    let ctrl_file = ts.dir.join("src/control.rs");
    std::fs::write(&ctrl_file, "fn multi_line() {\n    let y = 2;\n}\n").unwrap();
    let ctrl_req = json!({ "jsonrpc": "2.0", "id": 1030, "method": "textDocument/foldingRange", "params": { "textDocument": { "uri": to_uri(&ctrl_file) } } });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_ranges = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_ranges.len(), 1);
    assert_eq!(ctrl_ranges[0]["startLine"], 0);
    assert_eq!(ctrl_ranges[0]["endLine"], 2);
}

#[test]
fn test_folding_range_braces_inside_string_literals_ignored() {
    let ts = create_temp_shard("str_braces");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn has_str() {\n    let s = \"brace { in str }\";\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({ "jsonrpc": "2.0", "id": 104, "method": "textDocument/foldingRange", "params": { "textDocument": { "uri": to_uri(&file) } } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let ranges = resp["result"].as_array().unwrap();
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0]["startLine"], 0);
    assert_eq!(ranges[0]["endLine"], 2);
}

#[test]
fn test_folding_range_braces_inside_char_literals_ignored() {
    let ts = create_temp_shard("char_braces");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn has_chars() {\n    let c1 = '{';\n    let c2 = '}';\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({ "jsonrpc": "2.0", "id": 105, "method": "textDocument/foldingRange", "params": { "textDocument": { "uri": to_uri(&file) } } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let ranges = resp["result"].as_array().unwrap();
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0]["startLine"], 0);
    assert_eq!(ranges[0]["endLine"], 3);
}

#[test]
fn test_folding_range_braces_inside_line_comments_ignored() {
    let ts = create_temp_shard("line_comm");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn has_line_comment() {\n    // let x = {\n    let y = 42;\n    // };\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({ "jsonrpc": "2.0", "id": 106, "method": "textDocument/foldingRange", "params": { "textDocument": { "uri": to_uri(&file) } } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let ranges = resp["result"].as_array().unwrap();
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0]["startLine"], 0);
    assert_eq!(ranges[0]["endLine"], 4);
}

#[test]
fn test_folding_range_braces_inside_block_comments_ignored() {
    let ts = create_temp_shard("block_comm");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn has_block_comment() {\n    /*\n    {\n        commented block\n    }\n    */\n    let z = 99;\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({ "jsonrpc": "2.0", "id": 107, "method": "textDocument/foldingRange", "params": { "textDocument": { "uri": to_uri(&file) } } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let ranges = resp["result"].as_array().unwrap();
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0]["startLine"], 0);
    assert_eq!(ranges[0]["endLine"], 7);
}

#[test]
fn test_folding_range_escaped_quotes_inside_strings() {
    let ts = create_temp_shard("escaped_str");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn has_escaped() {\n    let s = \"quote \\\" { not block \\\" end\";\n    let b = 1;\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({ "jsonrpc": "2.0", "id": 108, "method": "textDocument/foldingRange", "params": { "textDocument": { "uri": to_uri(&file) } } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let ranges = resp["result"].as_array().unwrap();
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0]["startLine"], 0);
    assert_eq!(ranges[0]["endLine"], 3);
}

#[test]
fn test_folding_range_multiple_sequential_functions() {
    let ts = create_temp_shard("seq_fns");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn first() {\n    let a = 1;\n}\nfn second() {\n    let b = 2;\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({ "jsonrpc": "2.0", "id": 109, "method": "textDocument/foldingRange", "params": { "textDocument": { "uri": to_uri(&file) } } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let ranges = resp["result"].as_array().unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0]["startLine"], 0);
    assert_eq!(ranges[0]["endLine"], 2);
    assert_eq!(ranges[1]["startLine"], 3);
    assert_eq!(ranges[1]["endLine"], 5);
}

#[test]
fn test_folding_range_struct_impl_and_trait_blocks() {
    let ts = create_temp_shard("struct_impl");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "struct Data {\n    x: i32,\n}\nimpl Data {\n    fn new() -> Self {\n        Self { x: 0 }\n    }\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({ "jsonrpc": "2.0", "id": 118, "method": "textDocument/foldingRange", "params": { "textDocument": { "uri": to_uri(&file) } } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let ranges = resp["result"].as_array().unwrap();
    assert_eq!(ranges.len(), 3);
    assert_eq!(ranges[0]["startLine"], 0);
    assert_eq!(ranges[0]["endLine"], 2);
    assert_eq!(ranges[1]["startLine"], 4);
    assert_eq!(ranges[1]["endLine"], 6);
    assert_eq!(ranges[2]["startLine"], 3);
    assert_eq!(ranges[2]["endLine"], 7);
}
