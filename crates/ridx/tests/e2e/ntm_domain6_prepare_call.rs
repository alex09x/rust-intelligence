/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/prepareCallHierarchy (Domain 6, Part 1).
//! Tests call hierarchy preparation on functions, methods, overlay live buffers, and negative positions.

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

static CALL_COUNTER: AtomicU64 = AtomicU64::new(1);

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
    let id = CALL_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_prepcall_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let lib_str = dir.join("src/lib.rs").to_str().unwrap().to_string();
    let mut writer = IndexWriter::new(name, "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "compute".to_string(),
        path: format!("{}::compute", name),
        signature: Some("pub fn compute(x: i32) -> i32".to_string()),
        file: lib_str,
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 8,
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
fn test_prepare_call_hierarchy_single_free_function() {
    let ts = create_temp_shard("single_fn");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn compute(x: i32) -> i32 { x * 2 }\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 601, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 3 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 601);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "compute");
    assert_eq!(items[0]["kind"], 12); // SymbolKind::Function
    assert_eq!(items[0]["uri"], to_uri(&file));
}

#[test]
fn test_prepare_call_hierarchy_associated_function() {
    let id = CALL_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_prepcall_assoc_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("assoc_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::AssocFn,
        visibility: 1,
        is_macro_expansion: false,
        name: "new".to_string(),
        path: "assoc_crate::Foo::new".to_string(),
        signature: Some("pub fn new() -> Self".to_string()),
        file: lib_str,
        start_line: 3,
        start_col: 12,
        end_line: 3,
        end_col: 15,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "struct Foo;\nimpl Foo {\n    pub fn new() -> Self { Foo }\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 602, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 2, "character": 12 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "new");
    assert_eq!(items[0]["kind"], 6); // SymbolKind::Method
}

#[test]
fn test_prepare_call_hierarchy_method_with_signature() {
    let ts = create_temp_shard("sig");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn compute(x: i32) -> i32 { x }\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 603, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 4 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["detail"], "pub fn compute(x: i32) -> i32");
}

#[test]
fn test_prepare_call_hierarchy_overlay_live_buffer_precedence() {
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
        "jsonrpc": "2.0", "id": 604, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 4 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "live_fn");
}

#[test]
fn test_prepare_call_hierarchy_overlay_did_change_updates_symbol() {
    let ts = create_temp_shard("overlay_change");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn first_fn() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let change_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": to_uri(&file), "version": 2 },
            "contentChanges": [{ "text": "fn updated_fn() {}\n" }]
        }
    });
    assert!(dsp.dispatch(&change_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 605, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 4 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "updated_fn");
}

#[test]
fn test_prepare_call_hierarchy_overlay_did_close_falls_back() {
    let ts = create_temp_shard("overlay_close");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn compute() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn temp_fn() {}\n"
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
        "jsonrpc": "2.0", "id": 606, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 4 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "compute");
}

#[test]
fn test_prepare_call_hierarchy_cursor_on_non_function_returns_empty() {
    let ts = create_temp_shard("non_fn");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn compute() {}\nstruct Point { x: i32 }\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    // Pointing to struct (line 1)
    let req = json!({
        "jsonrpc": "2.0", "id": 607, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 1, "character": 7 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 0);

    // Positive control query on function at line 0
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 6070, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 3 }
        }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_items = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_items.len(), 1);
    assert_eq!(ctrl_items[0]["name"], "compute");
}
