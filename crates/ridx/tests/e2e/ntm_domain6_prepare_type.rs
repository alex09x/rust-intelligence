/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/prepareTypeHierarchy (Domain 6, Part 1).
//! Tests struct, enum, trait, type alias preparation, function filtering, and whitespace filtering.

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

static PREP_TYPE_COUNTER: AtomicU64 = AtomicU64::new(1);

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
    let id = PREP_TYPE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_preptype_{}_{}_{}", name, std::process::id(), id));
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
fn test_prepare_type_hierarchy_struct_definition() {
    let ts = create_temp_shard("struct_type");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "struct MyStruct;\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 681, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 7 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 681);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "MyStruct");
    assert_eq!(items[0]["kind"], 23); // SymbolKind::Struct
}

#[test]
fn test_prepare_type_hierarchy_enum_definition() {
    let id = PREP_TYPE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_preptype_enum_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("enum_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Enum, visibility: 1, is_macro_expansion: false,
        name: "Color".to_string(), path: "enum_crate::Color".to_string(),
        signature: Some("pub enum Color".to_string()), file: lib_str,
        start_line: 1, start_col: 1, end_line: 1, end_col: 25,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "enum Color { Red, Green, Blue }\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 682, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 5 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Color");
    assert_eq!(items[0]["kind"], 10); // SymbolKind::Enum
}

#[test]
fn test_prepare_type_hierarchy_trait_definition() {
    let id = PREP_TYPE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_preptype_trait_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("trait_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Trait, visibility: 1, is_macro_expansion: false,
        name: "Greeter".to_string(), path: "trait_crate::Greeter".to_string(),
        signature: Some("pub trait Greeter".to_string()), file: lib_str,
        start_line: 1, start_col: 1, end_line: 1, end_col: 8,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "trait Greeter { fn greet(&self); }\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 683, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 6 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Greeter");
    assert_eq!(items[0]["kind"], 11); // SymbolKind::Interface
}

#[test]
fn test_prepare_type_hierarchy_type_alias_definition() {
    let id = PREP_TYPE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_preptype_alias_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("alias_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::TyAlias, visibility: 1, is_macro_expansion: false,
        name: "Result".to_string(), path: "alias_crate::Result".to_string(),
        signature: Some("pub type Result<T> = std::result::Result<T, ()>".to_string()), file: lib_str,
        start_line: 1, start_col: 1, end_line: 1, end_col: 7,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "type Result<T> = std::result::Result<T, ()>;\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 684, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 5 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Result");
    assert_eq!(items[0]["kind"], 26); // SymbolKind::TypeParameter
}

#[test]
fn test_prepare_type_hierarchy_cursor_on_function_returns_empty() {
    let id = PREP_TYPE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_preptype_fn_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("fn_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "foo".to_string(), path: "fn_crate::foo".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 4,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "Bar".to_string(), path: "fn_crate::Bar".to_string(),
        signature: None, file: lib_str,
        start_line: 3, start_col: 1, end_line: 3, end_col: 15,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn foo() {}\n\nstruct Bar;\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    // Cursor on function `foo`
    let req = json!({
        "jsonrpc": "2.0", "id": 685, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 3 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 0);

    // Positive control on struct `Bar`
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 6850, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 2, "character": 7 }
        }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_items = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_items.len(), 1);
    assert_eq!(ctrl_items[0]["name"], "Bar");
}

#[test]
fn test_prepare_type_hierarchy_cursor_on_whitespace_returns_empty() {
    let ts = create_temp_shard("ws");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "struct MyStruct;\n\n\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 686, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 1, "character": 0 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 0);

    // Positive control on struct
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 6860, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 7 }
        }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_items = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_items.len(), 1);
}

#[test]
fn test_prepare_type_hierarchy_nonexistent_file_returns_empty() {
    let ts = create_temp_shard("missing_file");
    let file = ts.dir.join("src/missing.rs");
    let valid_file = ts.dir.join("src/lib.rs");
    std::fs::write(&valid_file, "struct MyStruct;\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 687, "method": "textDocument/prepareTypeHierarchy",
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
        "jsonrpc": "2.0", "id": 6870, "method": "textDocument/prepareTypeHierarchy",
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
