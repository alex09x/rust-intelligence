/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/codeLens (Domain 7, Part 1).
//! Tests definition lenses, overlay precedence, dirty buffer updates, and negative cases.

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

static LENS_COUNTER: AtomicU64 = AtomicU64::new(1);

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
    let id = LENS_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_lens_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let mut writer = IndexWriter::new(name, "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "test_fn".to_string(), path: format!("{}::test_fn", name),
        signature: None, file: "src/lib.rs".to_string(),
        start_line: 1, start_col: 4, end_line: 1, end_col: 11,
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
fn test_code_lens_disk_workspace_definition() {
    let ts = create_temp_shard("disk_def");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn test_fn() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 761, "method": "textDocument/codeLens",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 761);
    let lenses = resp["result"].as_array().unwrap();
    assert_eq!(lenses.len(), 1);
    assert_eq!(lenses[0]["range"]["start"]["line"], 0);
    assert_eq!(lenses[0]["range"]["start"]["character"], 3);
    assert_eq!(lenses[0]["range"]["end"]["character"], 10);
    assert_eq!(lenses[0]["data"]["name"], "test_fn");
}

#[test]
fn test_code_lens_multiple_definitions_in_file() {
    let id = LENS_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_lens_multi_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let mut writer = IndexWriter::new("multi_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "first_fn".to_string(), path: "multi_crate::first_fn".to_string(),
        signature: None, file: "src/lib.rs".to_string(),
        start_line: 1, start_col: 4, end_line: 1, end_col: 12,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "second_fn".to_string(), path: "multi_crate::second_fn".to_string(),
        signature: None, file: "src/lib.rs".to_string(),
        start_line: 5, start_col: 4, end_line: 5, end_col: 13,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    let file = dir.join("src/lib.rs");
    std::fs::write(&file, "fn first_fn() {}\n\n\n\nfn second_fn() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 762, "method": "textDocument/codeLens",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let lenses = resp["result"].as_array().unwrap();
    assert_eq!(lenses.len(), 2);
    assert_eq!(lenses[0]["data"]["name"], "first_fn");
    assert_eq!(lenses[1]["data"]["name"], "second_fn");
}

#[test]
fn test_code_lens_overlay_symbols_precedence() {
    let ts = create_temp_shard("overlay_prec");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn disk_fn() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn live_fn() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 763, "method": "textDocument/codeLens",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let lenses = resp["result"].as_array().unwrap();
    assert_eq!(lenses.len(), 1);
    assert_eq!(lenses[0]["data"]["name"], "live_fn");
}

#[test]
fn test_code_lens_overlay_did_change_updates_lenses() {
    let ts = create_temp_shard("overlay_change");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn initial_fn() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let change_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": to_uri(&file), "version": 2 },
            "contentChanges": [{ "text": "fn modified_fn() {}\n" }]
        }
    });
    assert!(dsp.dispatch(&change_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 764, "method": "textDocument/codeLens",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let lenses = resp["result"].as_array().unwrap();
    assert_eq!(lenses.len(), 1);
    assert_eq!(lenses[0]["data"]["name"], "modified_fn");
}

#[test]
fn test_code_lens_overlay_did_close_falls_back_to_index() {
    let ts = create_temp_shard("overlay_close");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn test_fn() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn temporary_fn() {}\n"
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
        "jsonrpc": "2.0", "id": 765, "method": "textDocument/codeLens",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let lenses = resp["result"].as_array().unwrap();
    assert_eq!(lenses.len(), 1);
    assert_eq!(lenses[0]["data"]["name"], "test_fn");
}

#[test]
fn test_code_lens_empty_file_returns_empty() {
    let ts = create_temp_shard("empty_file");
    let file = ts.dir.join("src/empty.rs");
    std::fs::write(&file, "").unwrap();
    let valid_file = ts.dir.join("src/lib.rs");
    std::fs::write(&valid_file, "fn test_fn() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 766, "method": "textDocument/codeLens",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let lenses = resp["result"].as_array().unwrap();
    assert_eq!(lenses.len(), 0);

    // Positive control query on file with definitions
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 7660, "method": "textDocument/codeLens",
        "params": { "textDocument": { "uri": to_uri(&valid_file) } }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_lenses = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_lenses.len(), 1);
}

#[test]
fn test_code_lens_file_without_symbols_returns_empty() {
    let ts = create_temp_shard("no_symbols");
    let file = ts.dir.join("src/other.rs");
    std::fs::write(&file, "// just comments\n// no functions\n").unwrap();
    let valid_file = ts.dir.join("src/lib.rs");
    std::fs::write(&valid_file, "fn test_fn() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 767, "method": "textDocument/codeLens",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let lenses = resp["result"].as_array().unwrap();
    assert_eq!(lenses.len(), 0);

    // Positive control query on valid file
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 7670, "method": "textDocument/codeLens",
        "params": { "textDocument": { "uri": to_uri(&valid_file) } }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_lenses = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_lenses.len(), 1);
}

#[test]
fn test_code_lens_nonexistent_file_returns_empty() {
    let ts = create_temp_shard("missing");
    let file = ts.dir.join("src/missing.rs");
    let valid_file = ts.dir.join("src/lib.rs");
    std::fs::write(&valid_file, "fn test_fn() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 768, "method": "textDocument/codeLens",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let lenses = resp["result"].as_array().unwrap();
    assert_eq!(lenses.len(), 0);

    // Positive control query on valid file
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 7680, "method": "textDocument/codeLens",
        "params": { "textDocument": { "uri": to_uri(&valid_file) } }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_lenses = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_lenses.len(), 1);
}

#[test]
fn test_code_lens_dispatcher_echoes_numeric_id() {
    let ts = create_temp_shard("numeric_id");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn test_fn() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 999333, "method": "textDocument/codeLens",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 999333);
    let lenses = resp["result"].as_array().unwrap();
    assert_eq!(lenses.len(), 1);
    assert_eq!(lenses[0]["range"]["start"]["line"], 0);
}

#[test]
fn test_code_lens_dispatcher_echoes_string_id() {
    let ts = create_temp_shard("string_id");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn test_fn() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": "lens-req-id-77", "method": "textDocument/codeLens",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], "lens-req-id-77");
    let lenses = resp["result"].as_array().unwrap();
    assert_eq!(lenses.len(), 1);
    assert_eq!(lenses[0]["range"]["start"]["line"], 0);
}
