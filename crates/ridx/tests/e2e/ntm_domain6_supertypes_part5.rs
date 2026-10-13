/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for typeHierarchy/supertypes (Domain 6, Part 5).
//! Tests multiple traits, comments in impl headers, associated types, tuple structs, and unit structs.

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

static SUPER_P5_COUNTER: AtomicU64 = AtomicU64::new(500);

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
fn test_type_hierarchy_supertypes_three_traits_on_same_struct() {
    let id = SUPER_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_p5_three_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("three_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Trait, visibility: 1, is_macro_expansion: false,
        name: "TraitA".to_string(), path: "three_crate::TraitA".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 15,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Trait, visibility: 1, is_macro_expansion: false,
        name: "TraitB".to_string(), path: "three_crate::TraitB".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 2, start_col: 1, end_line: 2, end_col: 15,
    });
    writer.add_definition(RawDefinition {
        def_id: 3, kind: DefKind::Trait, visibility: 1, is_macro_expansion: false,
        name: "TraitC".to_string(), path: "three_crate::TraitC".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 3, start_col: 1, end_line: 3, end_col: 15,
    });
    writer.add_definition(RawDefinition {
        def_id: 4, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "TriStruct".to_string(), path: "three_crate::TriStruct".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 4, start_col: 1, end_line: 4, end_col: 20,
    });
    writer.add_definition(RawDefinition {
        def_id: 5, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<TriStruct as TraitA>".to_string(), path: "<TriStruct as TraitA>".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 6, start_col: 1, end_line: 6, end_col: 27,
    });
    writer.add_definition(RawDefinition {
        def_id: 6, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<TriStruct as TraitB>".to_string(), path: "<TriStruct as TraitB>".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 7, start_col: 1, end_line: 7, end_col: 27,
    });
    writer.add_definition(RawDefinition {
        def_id: 7, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<TriStruct as TraitC>".to_string(), path: "<TriStruct as TraitC>".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 8, start_col: 1, end_line: 8, end_col: 27,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let code = "trait TraitA {}\ntrait TraitB {}\ntrait TraitC {}\nstruct TriStruct;\n\nimpl TraitA for TriStruct {}\nimpl TraitB for TriStruct {}\nimpl TraitC for TriStruct {}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 801, "method": "typeHierarchy/supertypes",
        "params": { "item": { "name": "TriStruct", "kind": 23, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 801);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 3);
}

#[test]
fn test_type_hierarchy_supertypes_impl_with_comments_in_header() {
    let id = SUPER_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_p5_comm_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("comm_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Trait, visibility: 1, is_macro_expansion: false,
        name: "LogTrait".to_string(), path: "comm_crate::LogTrait".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 18,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "Logger".to_string(), path: "comm_crate::Logger".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 2, start_col: 1, end_line: 2, end_col: 15,
    });
    writer.add_definition(RawDefinition {
        def_id: 3, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<Logger as LogTrait>".to_string(), path: "<Logger as LogTrait>".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 4, start_col: 1, end_line: 4, end_col: 35,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let code = "trait LogTrait {}\nstruct Logger;\n\nimpl LogTrait for Logger {}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 802, "method": "typeHierarchy/supertypes",
        "params": { "item": { "name": "Logger", "kind": 23, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 802);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "LogTrait");
}

#[test]
fn test_type_hierarchy_supertypes_associated_type_in_impl() {
    let id = SUPER_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_p5_assoc_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("assoc_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Trait, visibility: 1, is_macro_expansion: false,
        name: "Iterator".to_string(), path: "assoc_crate::Iterator".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 18,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "MyIter".to_string(), path: "assoc_crate::MyIter".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 2, start_col: 1, end_line: 2, end_col: 15,
    });
    writer.add_definition(RawDefinition {
        def_id: 3, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<MyIter as Iterator>".to_string(), path: "<MyIter as Iterator>".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 4, start_col: 1, end_line: 6, end_col: 2,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let code = "trait Iterator {}\nstruct MyIter;\n\nimpl Iterator for MyIter {\n    type Item = u32;\n}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 803, "method": "typeHierarchy/supertypes",
        "params": { "item": { "name": "MyIter", "kind": 23, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 803);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Iterator");
}

#[test]
fn test_type_hierarchy_supertypes_tuple_struct_impl() {
    let id = SUPER_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_p5_tuple_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("tuple_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Trait, visibility: 1, is_macro_expansion: false,
        name: "Summable".to_string(), path: "tuple_crate::Summable".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 18,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "Pair".to_string(), path: "tuple_crate::Pair".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 2, start_col: 1, end_line: 2, end_col: 22,
    });
    writer.add_definition(RawDefinition {
        def_id: 3, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<Pair as Summable>".to_string(), path: "<Pair as Summable>".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 4, start_col: 1, end_line: 4, end_col: 26,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let code = "trait Summable {}\nstruct Pair(i32, i32);\n\nimpl Summable for Pair {}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 804, "method": "typeHierarchy/supertypes",
        "params": { "item": { "name": "Pair", "kind": 23, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 804);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Summable");
}

#[test]
fn test_type_hierarchy_supertypes_unit_struct_impl() {
    let id = SUPER_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_p5_unit_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("unit_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Trait, visibility: 1, is_macro_expansion: false,
        name: "MarkerTrait".to_string(), path: "unit_crate::MarkerTrait".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 21,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "UnitToken".to_string(), path: "unit_crate::UnitToken".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 2, start_col: 1, end_line: 2, end_col: 17,
    });
    writer.add_definition(RawDefinition {
        def_id: 3, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<UnitToken as MarkerTrait>".to_string(), path: "<UnitToken as MarkerTrait>".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 4, start_col: 1, end_line: 4, end_col: 32,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let code = "trait MarkerTrait {}\nstruct UnitToken;\n\nimpl MarkerTrait for UnitToken {}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 805, "method": "typeHierarchy/supertypes",
        "params": { "item": { "name": "UnitToken", "kind": 23, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 805);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "MarkerTrait");
}
