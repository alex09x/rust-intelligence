/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/prepareTypeHierarchy (Domain 6, Part 2).
//! Tests request ID echoing, range equality, empty files, multi-type selection, and name boundaries.

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

static PREP_TYPE_P2_COUNTER: AtomicU64 = AtomicU64::new(100);

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
    let id = PREP_TYPE_P2_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_preptype_p2_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let lib_file = dir.join("src/lib.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new(name, "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "MyStruct".to_string(), path: format!("{}::MyStruct", name),
        signature: Some("pub struct MyStruct".to_string()), file: lib_str,
        start_line: 1, start_col: 1, end_line: 1, end_col: 18,
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
fn test_prepare_type_hierarchy_dispatcher_echoes_numeric_id() {
    let ts = create_temp_shard("num_id");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "struct MyStruct;\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 555888, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 7 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 555888);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "MyStruct");
}

#[test]
fn test_prepare_type_hierarchy_dispatcher_echoes_string_id() {
    let ts = create_temp_shard("str_id");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "struct MyStruct;\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": "preptype-str-77", "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 7 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], "preptype-str-77");
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "MyStruct");
}

#[test]
fn test_prepare_type_hierarchy_range_equals_selection_range() {
    let ts = create_temp_shard("range_eq");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "struct MyStruct;\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 688, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 7 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["range"], items[0]["selectionRange"]);
}

#[test]
fn test_prepare_type_hierarchy_empty_file_returns_empty() {
    let ts = create_temp_shard("empty_file");
    let file = ts.dir.join("src/empty.rs");
    std::fs::write(&file, "").unwrap();
    let valid_file = ts.dir.join("src/lib.rs");
    std::fs::write(&valid_file, "struct MyStruct;\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 689, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 0 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 0);

    // Positive control on valid file
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 6890, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&valid_file) },
            "position": { "line": 0, "character": 7 }
        }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_items = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_items.len(), 1);
}

#[test]
fn test_prepare_type_hierarchy_multiple_types_selection() {
    let id = PREP_TYPE_P2_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_preptype_multi_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("multi_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "FirstType".to_string(), path: "multi_crate::FirstType".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 10,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Enum, visibility: 1, is_macro_expansion: false,
        name: "SecondType".to_string(), path: "multi_crate::SecondType".to_string(),
        signature: None, file: lib_str,
        start_line: 3, start_col: 1, end_line: 3, end_col: 11,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "struct FirstType;\n\nenum SecondType { A, B }\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req1 = json!({
        "jsonrpc": "2.0", "id": 690, "method": "textDocument/prepareTypeHierarchy",
        "params": { "textDocument": { "uri": to_uri(&file) }, "position": { "line": 0, "character": 8 } }
    });
    assert!(dsp.dispatch(&req1));
    let resp1 = dsp.parse_response();
    let items1 = resp1["result"].as_array().unwrap();
    assert_eq!(items1.len(), 1);
    assert_eq!(items1[0]["name"], "FirstType");

    let req2 = json!({
        "jsonrpc": "2.0", "id": 691, "method": "textDocument/prepareTypeHierarchy",
        "params": { "textDocument": { "uri": to_uri(&file) }, "position": { "line": 2, "character": 6 } }
    });
    assert!(dsp.dispatch(&req2));
    let resp2 = dsp.parse_response();
    let items2 = resp2["result"].as_array().unwrap();
    assert_eq!(items2.len(), 1);
    assert_eq!(items2[0]["name"], "SecondType");
}

#[test]
fn test_prepare_type_hierarchy_position_at_type_name_start() {
    let ts = create_temp_shard("name_start");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "struct MyStruct;\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 692, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 0 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "MyStruct");
}

#[test]
fn test_prepare_type_hierarchy_position_at_type_name_end() {
    let ts = create_temp_shard("name_end");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "struct MyStruct;\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 693, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 8 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "MyStruct");
}
