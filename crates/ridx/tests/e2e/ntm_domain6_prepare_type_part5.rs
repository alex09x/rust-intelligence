/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/prepareTypeHierarchy (Domain 6, Part 5).
//! Tests cross-file references resolving to type definitions (struct, enum, trait, alias)
//! and filtering of references pointing to non-types (function).

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::path_to_uri;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static PREP_TYPE_P5_COUNTER: AtomicU64 = AtomicU64::new(500);

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
fn test_prepare_type_hierarchy_cross_file_reference_to_struct() {
    let id = PREP_TYPE_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_preptype_p5_ref_struct_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let lib_file = dir.join("src/lib.rs");
    let main_file = dir.join("src/main.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();
    let main_str = main_file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("ref_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 1,
        is_macro_expansion: false,
        name: "TargetStruct".to_string(),
        path: "ref_crate::TargetStruct".to_string(),
        signature: Some("pub struct TargetStruct".to_string()),
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 23,
    });
    writer.add_reference(RawReference {
        file: main_str.clone(),
        start_line: 1,
        start_col: 5,
        end_line: 1,
        end_col: 17,
        target_def_id: Some(1),
        target_path: "ref_crate::TargetStruct".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&lib_file, "pub struct TargetStruct;\n").unwrap();
    std::fs::write(&main_file, "use TargetStruct;\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 766, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&main_file) },
            "position": { "line": 0, "character": 6 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 766);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "TargetStruct");
    assert_eq!(items[0]["kind"], 23);
    assert_eq!(items[0]["uri"], to_uri(&lib_file));
}

#[test]
fn test_prepare_type_hierarchy_cross_file_reference_to_enum() {
    let id = PREP_TYPE_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_preptype_p5_ref_enum_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let lib_file = dir.join("src/lib.rs");
    let main_file = dir.join("src/main.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();
    let main_str = main_file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("ref_enum_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Enum,
        visibility: 1,
        is_macro_expansion: false,
        name: "TargetEnum".to_string(),
        path: "ref_enum_crate::TargetEnum".to_string(),
        signature: Some("pub enum TargetEnum".to_string()),
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 21,
    });
    writer.add_reference(RawReference {
        file: main_str.clone(),
        start_line: 1,
        start_col: 5,
        end_line: 1,
        end_col: 15,
        target_def_id: Some(1),
        target_path: "ref_enum_crate::TargetEnum".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Enum,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&lib_file, "pub enum TargetEnum { A, B }\n").unwrap();
    std::fs::write(&main_file, "use TargetEnum;\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 767, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&main_file) },
            "position": { "line": 0, "character": 6 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 767);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "TargetEnum");
    assert_eq!(items[0]["kind"], 10);
    assert_eq!(items[0]["uri"], to_uri(&lib_file));
}

#[test]
fn test_prepare_type_hierarchy_cross_file_reference_to_trait() {
    let id = PREP_TYPE_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_preptype_p5_ref_trait_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let lib_file = dir.join("src/lib.rs");
    let main_file = dir.join("src/main.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();
    let main_str = main_file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("ref_trait_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 1,
        is_macro_expansion: false,
        name: "TargetTrait".to_string(),
        path: "ref_trait_crate::TargetTrait".to_string(),
        signature: Some("pub trait TargetTrait".to_string()),
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 22,
    });
    writer.add_reference(RawReference {
        file: main_str.clone(),
        start_line: 1,
        start_col: 5,
        end_line: 1,
        end_col: 16,
        target_def_id: Some(1),
        target_path: "ref_trait_crate::TargetTrait".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Trait,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&lib_file, "pub trait TargetTrait {}\n").unwrap();
    std::fs::write(&main_file, "use TargetTrait;\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 768, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&main_file) },
            "position": { "line": 0, "character": 6 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 768);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "TargetTrait");
    assert_eq!(items[0]["kind"], 11);
    assert_eq!(items[0]["uri"], to_uri(&lib_file));
}

#[test]
fn test_prepare_type_hierarchy_cross_file_reference_to_type_alias() {
    let id = PREP_TYPE_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_preptype_p5_ref_alias_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let lib_file = dir.join("src/lib.rs");
    let main_file = dir.join("src/main.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();
    let main_str = main_file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("ref_alias_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::TyAlias,
        visibility: 1,
        is_macro_expansion: false,
        name: "TargetAlias".to_string(),
        path: "ref_alias_crate::TargetAlias".to_string(),
        signature: Some("pub type TargetAlias = i64".to_string()),
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 22,
    });
    writer.add_reference(RawReference {
        file: main_str.clone(),
        start_line: 1,
        start_col: 5,
        end_line: 1,
        end_col: 16,
        target_def_id: Some(1),
        target_path: "ref_alias_crate::TargetAlias".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::TyAlias,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&lib_file, "pub type TargetAlias = i64;\n").unwrap();
    std::fs::write(&main_file, "use TargetAlias;\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 769, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&main_file) },
            "position": { "line": 0, "character": 6 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 769);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "TargetAlias");
    assert_eq!(items[0]["kind"], 26);
    assert_eq!(items[0]["uri"], to_uri(&lib_file));
}

#[test]
fn test_prepare_type_hierarchy_reference_to_function_returns_empty() {
    let id = PREP_TYPE_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_preptype_p5_ref_fn_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let lib_file = dir.join("src/lib.rs");
    let main_file = dir.join("src/main.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();
    let main_str = main_file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("ref_fn_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "target_func".to_string(),
        path: "ref_fn_crate::target_func".to_string(),
        signature: Some("pub fn target_func()".to_string()),
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 20,
    });
    writer.add_reference(RawReference {
        file: main_str.clone(),
        start_line: 1,
        start_col: 5,
        end_line: 1,
        end_col: 16,
        target_def_id: Some(1),
        target_path: "ref_fn_crate::target_func".to_string(),
        target_file: Some(lib_str),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&lib_file, "pub fn target_func() {}\n").unwrap();
    std::fs::write(&main_file, "use target_func;\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 770, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&main_file) },
            "position": { "line": 0, "character": 6 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 770);
    assert_eq!(resp["result"], json!([]));
}
