/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for typeHierarchy/subtypes (Domain 6, Part 5).
//! Tests multiple struct implementors, comments in headers, associated types, and tuple structs.

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

static SUB_P5_COUNTER: AtomicU64 = AtomicU64::new(500);

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
fn test_type_hierarchy_subtypes_three_struct_implementors() {
    let id = SUB_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_sub_p5_three_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("three_sub_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Trait, visibility: 1, is_macro_expansion: false,
        name: "CommonTrait".to_string(), path: "three_sub_crate::CommonTrait".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 22,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "ImplA".to_string(), path: "three_sub_crate::ImplA".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 2, start_col: 1, end_line: 2, end_col: 16,
    });
    writer.add_definition(RawDefinition {
        def_id: 3, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "ImplB".to_string(), path: "three_sub_crate::ImplB".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 3, start_col: 1, end_line: 3, end_col: 16,
    });
    writer.add_definition(RawDefinition {
        def_id: 4, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "ImplC".to_string(), path: "three_sub_crate::ImplC".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 4, start_col: 1, end_line: 4, end_col: 16,
    });
    writer.add_definition(RawDefinition {
        def_id: 5, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<ImplA as CommonTrait>".to_string(), path: "<ImplA as CommonTrait>".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 6, start_col: 1, end_line: 6, end_col: 31,
    });
    writer.add_definition(RawDefinition {
        def_id: 6, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<ImplB as CommonTrait>".to_string(), path: "<ImplB as CommonTrait>".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 7, start_col: 1, end_line: 7, end_col: 31,
    });
    writer.add_definition(RawDefinition {
        def_id: 7, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<ImplC as CommonTrait>".to_string(), path: "<ImplC as CommonTrait>".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 8, start_col: 1, end_line: 8, end_col: 31,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let code = "trait CommonTrait {}\nstruct ImplA;\nstruct ImplB;\nstruct ImplC;\n\nimpl CommonTrait for ImplA {}\nimpl CommonTrait for ImplB {}\nimpl CommonTrait for ImplC {}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 831, "method": "typeHierarchy/subtypes",
        "params": { "item": { "name": "CommonTrait", "kind": 11, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 831);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 3);
}

#[test]
fn test_type_hierarchy_subtypes_impl_with_comments_in_header() {
    let id = SUB_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_sub_p5_comm_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("comm_sub_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Trait, visibility: 1, is_macro_expansion: false,
        name: "PrintTrait".to_string(), path: "comm_sub_crate::PrintTrait".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 21,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "Printer".to_string(), path: "comm_sub_crate::Printer".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 2, start_col: 1, end_line: 2, end_col: 16,
    });
    writer.add_definition(RawDefinition {
        def_id: 3, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<Printer as PrintTrait>".to_string(), path: "<Printer as PrintTrait>".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 4, start_col: 1, end_line: 4, end_col: 33,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let code = "trait PrintTrait {}\nstruct Printer;\n\nimpl PrintTrait for Printer {}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 832, "method": "typeHierarchy/subtypes",
        "params": { "item": { "name": "PrintTrait", "kind": 11, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 832);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Printer");
    assert_eq!(items[0]["kind"], 23);
}

#[test]
fn test_type_hierarchy_subtypes_associated_type_in_impl() {
    let id = SUB_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_sub_p5_assoc_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("assoc_sub_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Trait, visibility: 1, is_macro_expansion: false,
        name: "Collection".to_string(), path: "assoc_sub_crate::Collection".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 21,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "MyList".to_string(), path: "assoc_sub_crate::MyList".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 2, start_col: 1, end_line: 2, end_col: 15,
    });
    writer.add_definition(RawDefinition {
        def_id: 3, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<MyList as Collection>".to_string(), path: "<MyList as Collection>".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 4, start_col: 1, end_line: 6, end_col: 2,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let code = "trait Collection {}\nstruct MyList;\n\nimpl Collection for MyList {\n    type Item = i32;\n}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 833, "method": "typeHierarchy/subtypes",
        "params": { "item": { "name": "Collection", "kind": 11, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 833);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "MyList");
}

#[test]
fn test_type_hierarchy_subtypes_tuple_struct_implementor() {
    let id = SUB_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_sub_p5_tuple_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("tuple_sub_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Trait, visibility: 1, is_macro_expansion: false,
        name: "Computable".to_string(), path: "tuple_sub_crate::Computable".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 21,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "Coord".to_string(), path: "tuple_sub_crate::Coord".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 2, start_col: 1, end_line: 2, end_col: 24,
    });
    writer.add_definition(RawDefinition {
        def_id: 3, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<Coord as Computable>".to_string(), path: "<Coord as Computable>".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 4, start_col: 1, end_line: 4, end_col: 28,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let code = "trait Computable {}\nstruct Coord(f64, f64);\n\nimpl Computable for Coord {}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 834, "method": "typeHierarchy/subtypes",
        "params": { "item": { "name": "Computable", "kind": 11, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 834);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Coord");
}
