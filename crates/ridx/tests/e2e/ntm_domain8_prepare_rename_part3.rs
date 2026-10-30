/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/prepareRename (Domain 8, Part 3).
//! Tests index-backed symbol declarations for structs, functions, enums, traits, aliases, and constants.

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

static PREP_P3_COUNTER: AtomicU64 = AtomicU64::new(1);

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
fn test_prepare_rename_index_struct_definition_exact_range() {
    let id = PREP_P3_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_prep_p3_struct_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("test_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 1,
        is_macro_expansion: false,
        name: "MyStruct".to_string(),
        path: "test_crate::MyStruct".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 16,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "pub struct MyStruct {\n    val: i32,\n}\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 301, "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 12 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 301);
    assert_eq!(resp["result"]["placeholder"], "MyStruct");
    assert_eq!(resp["result"]["range"]["start"]["line"], 0);
    assert_eq!(resp["result"]["range"]["start"]["character"], 11);
    assert_eq!(resp["result"]["range"]["end"]["line"], 0);
    assert_eq!(resp["result"]["range"]["end"]["character"], 19);
}

#[test]
fn test_prepare_rename_index_fn_definition_exact_range() {
    let id = PREP_P3_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_prep_p3_fn_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("test_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "compute_sum".to_string(),
        path: "test_crate::compute_sum".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 4,
        end_line: 1,
        end_col: 15,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn compute_sum(a: i32, b: i32) -> i32 { a + b }\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 302, "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 5 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 302);
    assert_eq!(resp["result"]["placeholder"], "compute_sum");
    assert_eq!(resp["result"]["range"]["start"]["line"], 0);
    assert_eq!(resp["result"]["range"]["start"]["character"], 3);
    assert_eq!(resp["result"]["range"]["end"]["line"], 0);
    assert_eq!(resp["result"]["range"]["end"]["character"], 14);
}

#[test]
fn test_prepare_rename_index_enum_definition_exact_range() {
    let id = PREP_P3_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_prep_p3_enum_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("test_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Enum,
        visibility: 1,
        is_macro_expansion: false,
        name: "State".to_string(),
        path: "test_crate::State".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 6,
        end_line: 1,
        end_col: 11,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "enum State { On, Off }\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 303, "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 7 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 303);
    assert_eq!(resp["result"]["placeholder"], "State");
    assert_eq!(resp["result"]["range"]["start"]["line"], 0);
    assert_eq!(resp["result"]["range"]["start"]["character"], 5);
    assert_eq!(resp["result"]["range"]["end"]["line"], 0);
    assert_eq!(resp["result"]["range"]["end"]["character"], 10);
}

#[test]
fn test_prepare_rename_index_trait_definition_exact_range() {
    let id = PREP_P3_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_prep_p3_trait_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("test_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 1,
        is_macro_expansion: false,
        name: "Renderable".to_string(),
        path: "test_crate::Renderable".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 7,
        end_line: 1,
        end_col: 17,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "pub trait Renderable {}\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 304, "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 12 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 304);
    assert_eq!(resp["result"]["placeholder"], "Renderable");
    assert_eq!(resp["result"]["range"]["start"]["line"], 0);
    assert_eq!(resp["result"]["range"]["start"]["character"], 10);
    assert_eq!(resp["result"]["range"]["end"]["character"], 20);
}

#[test]
fn test_prepare_rename_index_type_alias_exact_range() {
    let id = PREP_P3_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_prep_p3_alias_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("test_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::TyAlias,
        visibility: 1,
        is_macro_expansion: false,
        name: "IntList".to_string(),
        path: "test_crate::IntList".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 6,
        end_line: 1,
        end_col: 13,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "type IntList = Vec<i32>;\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 305, "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 8 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 305);
    assert_eq!(resp["result"]["placeholder"], "IntList");
    assert_eq!(resp["result"]["range"]["start"]["character"], 5);
    assert_eq!(resp["result"]["range"]["end"]["character"], 12);
}

#[test]
fn test_prepare_rename_index_const_definition_exact_range() {
    let id = PREP_P3_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_prep_p3_const_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("test_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Const,
        visibility: 1,
        is_macro_expansion: false,
        name: "MAX_CAP".to_string(),
        path: "test_crate::MAX_CAP".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 7,
        end_line: 1,
        end_col: 14,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "const MAX_CAP: usize = 1024;\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 306, "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 9 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 306);
    assert_eq!(resp["result"]["placeholder"], "MAX_CAP");
    assert_eq!(resp["result"]["range"]["start"]["character"], 6);
    assert_eq!(resp["result"]["range"]["end"]["character"], 13);
}
