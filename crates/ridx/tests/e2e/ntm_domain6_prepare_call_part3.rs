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
        def_id: 1, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "compute".to_string(), path: format!("{}::compute", name),
        signature: Some("pub fn compute(x: i32) -> i32".to_string()),
        file: lib_str,
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
fn test_prepare_call_hierarchy_multiple_functions_selection() {
    let id = CALL_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_prepcall_p2_multi_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let mut writer = IndexWriter::new("multi_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "func_a".to_string(), path: "multi_crate::func_a".to_string(),
        signature: None, file: "src/lib.rs".to_string(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 7,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "func_b".to_string(), path: "multi_crate::func_b".to_string(),
        signature: None, file: "src/lib.rs".to_string(),
        start_line: 3, start_col: 1, end_line: 3, end_col: 7,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    let file = dir.join("src/lib.rs");
    std::fs::write(&file, "fn func_a() {}\n\nfn func_b() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    // Query on func_b
    let req = json!({
        "jsonrpc": "2.0", "id": 615, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 2, "character": 3 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "func_b");
}

#[test]
fn test_prepare_call_hierarchy_position_at_fn_name_start() {
    let ts = create_temp_shard("start_pos");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn compute() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 616, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 0 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "compute");
}

#[test]
fn test_prepare_call_hierarchy_position_at_fn_name_end() {
    let ts = create_temp_shard("end_pos");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn compute() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 617, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 6 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "compute");
}

#[test]
fn test_prepare_call_hierarchy_position_beyond_eof_returns_empty() {
    let ts = create_temp_shard("beyond_eof");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn target() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 618, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 100, "character": 0 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 0);

    // Positive control query on target
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 6180, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 3 }
        }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_items = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_items.len(), 1);
}

#[test]
fn test_prepare_call_hierarchy_missing_position_param_returns_empty() {
    let ts = create_temp_shard("missing_param");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn target() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 619, "method": "textDocument/prepareCallHierarchy",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 0);

    // Positive control query with position
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 6190, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 3 }
        }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_items = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_items.len(), 1);
}

#[test]
fn test_prepare_call_hierarchy_sequential_requests_isolation() {
    let ts = create_temp_shard("seq");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn target() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req1 = json!({
        "jsonrpc": "2.0", "id": 1, "method": "textDocument/prepareCallHierarchy",
        "params": { "textDocument": { "uri": to_uri(&file) }, "position": { "line": 0, "character": 3 } }
    });
    assert!(dsp.dispatch(&req1));
    let resp1 = dsp.parse_response();
    assert_eq!(resp1["id"], 1);
    assert_eq!(resp1["result"].as_array().unwrap().len(), 1);

    let req2 = json!({
        "jsonrpc": "2.0", "id": 2, "method": "textDocument/prepareCallHierarchy",
        "params": { "textDocument": { "uri": to_uri(&file) }, "position": { "line": 0, "character": 3 } }
    });
    assert!(dsp.dispatch(&req2));
    let resp2 = dsp.parse_response();
    assert_eq!(resp2["id"], 2);
    assert_eq!(resp2["result"].as_array().unwrap().len(), 1);
}
