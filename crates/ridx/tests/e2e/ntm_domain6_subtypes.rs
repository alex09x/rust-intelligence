/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for typeHierarchy/subtypes (Domain 6, Part 1).
//! Tests single/multiple struct implementors, enum implementors, generics, and zero subtypes.

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

static SUB_COUNTER: AtomicU64 = AtomicU64::new(1);

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
    let id = SUB_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_sub_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let lib_file = dir.join("src/lib.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new(name, "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Trait, visibility: 1, is_macro_expansion: false,
        name: "Greeter".to_string(), path: format!("{}::Greeter", name),
        signature: Some("pub trait Greeter".to_string()), file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 18,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "Robot".to_string(), path: format!("{}::Robot", name),
        signature: Some("pub struct Robot".to_string()), file: lib_str.clone(),
        start_line: 3, start_col: 1, end_line: 3, end_col: 17,
    });
    writer.add_definition(RawDefinition {
        def_id: 3, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<Robot as Greeter>".to_string(), path: "<Robot as Greeter>".to_string(),
        signature: Some("impl Greeter for Robot".to_string()), file: lib_str,
        start_line: 5, start_col: 1, end_line: 7, end_col: 2,
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
fn test_type_hierarchy_subtypes_single_struct_implementor() {
    let ts = create_temp_shard("single");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "trait Greeter {}\n\nstruct Robot;\n\nimpl Greeter for Robot {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 731, "method": "typeHierarchy/subtypes",
        "params": {
            "item": { "name": "Greeter", "kind": 11, "uri": to_uri(&file) }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 731);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Robot");
    assert_eq!(items[0]["kind"], 23); // SymbolKind::Struct
}

#[test]
fn test_type_hierarchy_subtypes_multiple_struct_implementors() {
    let id = SUB_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_sub_m_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("multi_sub_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Trait, visibility: 1, is_macro_expansion: false,
        name: "Greeter".to_string(), path: "multi_sub_crate::Greeter".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 18,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "Robot".to_string(), path: "multi_sub_crate::Robot".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 3, start_col: 1, end_line: 3, end_col: 17,
    });
    writer.add_definition(RawDefinition {
        def_id: 3, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "Human".to_string(), path: "multi_sub_crate::Human".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 4, start_col: 1, end_line: 4, end_col: 17,
    });
    writer.add_definition(RawDefinition {
        def_id: 4, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<Robot as Greeter>".to_string(), path: "<Robot as Greeter>".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 6, start_col: 1, end_line: 6, end_col: 25,
    });
    writer.add_definition(RawDefinition {
        def_id: 5, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<Human as Greeter>".to_string(), path: "<Human as Greeter>".to_string(),
        signature: None, file: lib_str,
        start_line: 7, start_col: 1, end_line: 7, end_col: 25,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "trait Greeter {}\n\nstruct Robot;\nstruct Human;\n\nimpl Greeter for Robot {}\nimpl Greeter for Human {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 732, "method": "typeHierarchy/subtypes",
        "params": {
            "item": { "name": "Greeter", "kind": 11, "uri": to_uri(&file) }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    let names: Vec<&str> = items.iter().map(|it| it["name"].as_str().unwrap()).collect();
    assert_eq!(names[0], "Robot");
    assert_eq!(names[1], "Human");
}

#[test]
fn test_type_hierarchy_subtypes_enum_implementor() {
    let id = SUB_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_sub_enum_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("enum_sub_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Trait, visibility: 1, is_macro_expansion: false,
        name: "Describable".to_string(), path: "enum_sub_crate::Describable".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 22,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Enum, visibility: 1, is_macro_expansion: false,
        name: "Status".to_string(), path: "enum_sub_crate::Status".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 3, start_col: 1, end_line: 3, end_col: 18,
    });
    writer.add_definition(RawDefinition {
        def_id: 3, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<Status as Describable>".to_string(), path: "<Status as Describable>".to_string(),
        signature: None, file: lib_str,
        start_line: 5, start_col: 1, end_line: 5, end_col: 35,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "trait Describable {}\n\nenum Status { Ok, Err }\n\nimpl Describable for Status {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 733, "method": "typeHierarchy/subtypes",
        "params": {
            "item": { "name": "Describable", "kind": 11, "uri": to_uri(&file) }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Status");
    assert_eq!(items[0]["kind"], 10); // SymbolKind::Enum
}

#[test]
fn test_type_hierarchy_subtypes_generic_struct_implementor() {
    let id = SUB_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_sub_gs_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("gs_sub_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Trait, visibility: 1, is_macro_expansion: false,
        name: "Container".to_string(), path: "gs_sub_crate::Container".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 20,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "Wrapper".to_string(), path: "gs_sub_crate::Wrapper".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 3, start_col: 1, end_line: 3, end_col: 20,
    });
    writer.add_definition(RawDefinition {
        def_id: 3, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<Wrapper as Container>".to_string(), path: "<Wrapper as Container>".to_string(),
        signature: None, file: lib_str,
        start_line: 5, start_col: 1, end_line: 5, end_col: 40,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "trait Container {}\n\nstruct Wrapper<T>(T);\n\nimpl<T> Container for Wrapper<T> {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 734, "method": "typeHierarchy/subtypes",
        "params": {
            "item": { "name": "Container", "kind": 11, "uri": to_uri(&file) }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Wrapper");
}

#[test]
fn test_type_hierarchy_subtypes_zero_subtypes_returns_empty() {
    let ts = create_temp_shard("zero");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "trait Greeter {}\ntrait LoneTrait {}\n\nstruct Robot;\n\nimpl Greeter for Robot {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    // LoneTrait has no implementors
    let req = json!({
        "jsonrpc": "2.0", "id": 735, "method": "typeHierarchy/subtypes",
        "params": {
            "item": { "name": "LoneTrait", "kind": 11, "uri": to_uri(&file) }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 0);

    // Positive control on Greeter
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 7350, "method": "typeHierarchy/subtypes",
        "params": {
            "item": { "name": "Greeter", "kind": 11, "uri": to_uri(&file) }
        }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_items = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_items.len(), 1);
    assert_eq!(ctrl_items[0]["name"], "Robot");
}
