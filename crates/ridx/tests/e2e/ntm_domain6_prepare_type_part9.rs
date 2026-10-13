/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/prepareTypeHierarchy (Domain 6, Part 9).
//! Tests multi-root routing, exact item schema shape, submodule files, consecutive queries, and complex type alias signatures.

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

static PREP_TYPE_P9_COUNTER: AtomicU64 = AtomicU64::new(900);

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

    fn new_multi(shards: Vec<PathBuf>) -> Self {
        let mut d = Self {
            lifecycle: LifecycleManager::new(),
            multi_root: MultiRootWorkspace::new(),
            overlay_mgr: OverlayManager::new(),
            cancelled: Arc::new(Mutex::new(HashSet::new())),
            writer: Vec::new(),
        };
        for s in shards {
            assert!(d.multi_root.add_root(s));
        }
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
fn test_prepare_type_hierarchy_multi_root_workspace_routing() {
    let id = PREP_TYPE_P9_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir1 = std::env::temp_dir().join(format!("ridx_preptype_p9_mr1_{}_{}", std::process::id(), id));
    let dir2 = std::env::temp_dir().join(format!("ridx_preptype_p9_mr2_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir1.join("src")).unwrap();
    std::fs::create_dir_all(dir2.join("src")).unwrap();
    let shard1 = dir1.join("index.ridx");
    let shard2 = dir2.join("index.ridx");
    let file2 = dir2.join("src/lib.rs");
    let file2_str = file2.to_str().unwrap().to_string();

    let mut writer1 = IndexWriter::new("root1_crate", "0.1.0");
    writer1.write_to(&mut File::create(&shard1).unwrap()).unwrap();

    let mut writer2 = IndexWriter::new("root2_crate", "0.1.0");
    writer2.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "Root2Struct".to_string(), path: "root2_crate::Root2Struct".to_string(),
        signature: Some("pub struct Root2Struct".to_string()), file: file2_str,
        start_line: 1, start_col: 1, end_line: 1, end_col: 22,
    });
    writer2.write_to(&mut File::create(&shard2).unwrap()).unwrap();
    std::fs::write(&file2, "pub struct Root2Struct;\n").unwrap();

    let mut dsp = TestDispatcher::new_multi(vec![shard1, shard2]);

    let req = json!({
        "jsonrpc": "2.0", "id": 786, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file2) },
            "position": { "line": 0, "character": 12 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 786);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Root2Struct");
}

#[test]
fn test_prepare_type_hierarchy_item_shape_exact_keys() {
    let id = PREP_TYPE_P9_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_preptype_p9_shape_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("shape_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "ShapeStruct".to_string(), path: "shape_crate::ShapeStruct".to_string(),
        signature: Some("pub struct ShapeStruct".to_string()), file: lib_str,
        start_line: 1, start_col: 1, end_line: 1, end_col: 23,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "pub struct ShapeStruct;\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 787, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 13 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 787);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    let item = &items[0];

    assert_eq!(item["name"], "ShapeStruct");
    assert_eq!(item["kind"], 23);
    assert_eq!(item["detail"], "pub struct ShapeStruct");
    assert_eq!(item["uri"], to_uri(&file));
    assert_eq!(item["range"]["start"]["line"], 0);
    assert_eq!(item["range"]["start"]["character"], 0);
    assert_eq!(item["range"]["end"]["line"], 0);
    assert_eq!(item["range"]["end"]["character"], 22);
    assert_eq!(item["selectionRange"], item["range"]);
}

#[test]
fn test_prepare_type_hierarchy_nested_struct_in_submodule_file() {
    let id = PREP_TYPE_P9_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_preptype_p9_submod_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src/models")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/models/user.rs");
    let file_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("submod_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "User".to_string(), path: "submod_crate::models::user::User".to_string(),
        signature: Some("pub struct User".to_string()), file: file_str,
        start_line: 1, start_col: 1, end_line: 1, end_col: 16,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "pub struct User;\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 788, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 12 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 788);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "User");
}

#[test]
fn test_prepare_type_hierarchy_consecutive_requests_different_types() {
    let id = PREP_TYPE_P9_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_preptype_p9_consec_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("consec_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "ItemAlpha".to_string(), path: "consec_crate::ItemAlpha".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 16,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Enum, visibility: 1, is_macro_expansion: false,
        name: "ItemBeta".to_string(), path: "consec_crate::ItemBeta".to_string(),
        signature: None, file: lib_str,
        start_line: 3, start_col: 1, end_line: 3, end_col: 15,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "struct ItemAlpha;\n\nenum ItemBeta {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    // First request: struct at line 0
    let req1 = json!({
        "jsonrpc": "2.0", "id": 789, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 8 }
        }
    });
    assert!(dsp.dispatch(&req1));
    let resp1 = dsp.parse_response();
    assert_eq!(resp1["id"], 789);
    let items1 = resp1["result"].as_array().unwrap();
    assert_eq!(items1.len(), 1);
    assert_eq!(items1[0]["name"], "ItemAlpha");
    assert_eq!(items1[0]["kind"], 23);

    // Second request: enum at line 2
    let req2 = json!({
        "jsonrpc": "2.0", "id": 790, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 2, "character": 6 }
        }
    });
    assert!(dsp.dispatch(&req2));
    let resp2 = dsp.parse_response();
    assert_eq!(resp2["id"], 790);
    let items2 = resp2["result"].as_array().unwrap();
    assert_eq!(items2.len(), 1);
    assert_eq!(items2[0]["name"], "ItemBeta");
    assert_eq!(items2[0]["kind"], 10);
}

#[test]
fn test_prepare_type_hierarchy_type_alias_with_complex_signature() {
    let id = PREP_TYPE_P9_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_preptype_p9_alias_sig_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let sig = "pub type DynResult<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>";
    let mut writer = IndexWriter::new("dyn_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::TyAlias, visibility: 1, is_macro_expansion: false,
        name: "DynResult".to_string(), path: "dyn_crate::DynResult".to_string(),
        signature: Some(sig.to_string()), file: lib_str,
        start_line: 1, start_col: 1, end_line: 1, end_col: 22,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, format!("{};\n", sig)).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 791, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 9 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 791);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "DynResult");
    assert_eq!(items[0]["detail"], sig);
    assert_eq!(items[0]["kind"], 26);
}
