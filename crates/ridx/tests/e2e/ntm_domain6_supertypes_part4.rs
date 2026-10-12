/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for typeHierarchy/supertypes (Domain 6, Part 4).
//! Tests where-clauses, qualified paths, inherent impls, isolation, and fallback ranges.

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

static SUPER_P4_COUNTER: AtomicU64 = AtomicU64::new(300);

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
fn test_type_hierarchy_supertypes_where_clause_syntax() {
    let id = SUPER_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_where_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("where_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Trait, visibility: 1, is_macro_expansion: false,
        name: "Serializable".to_string(), path: "where_crate::Serializable".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 25,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "Boxed".to_string(), path: "where_crate::Boxed".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 3, start_col: 1, end_line: 3, end_col: 20,
    });
    writer.add_definition(RawDefinition {
        def_id: 3, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<Boxed as Serializable>".to_string(), path: "<Boxed as Serializable>".to_string(),
        signature: None, file: lib_str,
        start_line: 5, start_col: 1, end_line: 7, end_col: 2,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "trait Serializable {}\n\nstruct Boxed<T>(T);\n\nimpl<T> Serializable for Boxed<T> where T: Clone {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 714, "method": "typeHierarchy/supertypes",
        "params": {
            "item": { "name": "Boxed", "kind": 23, "uri": to_uri(&file) }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Serializable");
}

#[test]
fn test_type_hierarchy_supertypes_qualified_trait_path() {
    let id = SUPER_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_qp_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("qp_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Trait, visibility: 1, is_macro_expansion: false,
        name: "Greeter".to_string(), path: "qp_crate::Greeter".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 18,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "Robot".to_string(), path: "qp_crate::Robot".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 3, start_col: 1, end_line: 3, end_col: 17,
    });
    writer.add_definition(RawDefinition {
        def_id: 3, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<Robot as Greeter>".to_string(), path: "<Robot as Greeter>".to_string(),
        signature: None, file: lib_str,
        start_line: 5, start_col: 1, end_line: 5, end_col: 40,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "trait Greeter {}\n\nstruct Robot;\n\nimpl qp_crate::Greeter for Robot {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 715, "method": "typeHierarchy/supertypes",
        "params": {
            "item": { "name": "Robot", "kind": 23, "uri": to_uri(&file) }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "qp_crate::Greeter");
}

#[test]
fn test_type_hierarchy_supertypes_inherent_impl_filtered() {
    let id = SUPER_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_inh_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("inh_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "Robot".to_string(), path: "inh_crate::Robot".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 17,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "Robot".to_string(), path: "inh_crate::Robot".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 3, start_col: 1, end_line: 5, end_col: 2,
    });
    writer.add_definition(RawDefinition {
        def_id: 3, kind: DefKind::Trait, visibility: 1, is_macro_expansion: false,
        name: "Worker".to_string(), path: "inh_crate::Worker".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 7, start_col: 1, end_line: 7, end_col: 17,
    });
    writer.add_definition(RawDefinition {
        def_id: 4, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "Cyborg".to_string(), path: "inh_crate::Cyborg".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 9, start_col: 1, end_line: 9, end_col: 18,
    });
    writer.add_definition(RawDefinition {
        def_id: 5, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<Cyborg as Worker>".to_string(), path: "<Cyborg as Worker>".to_string(),
        signature: None, file: lib_str,
        start_line: 11, start_col: 1, end_line: 11, end_col: 30,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "struct Robot;\n\nimpl Robot {\n    fn tick(&self) {}\n}\n\ntrait Worker {}\nstruct Cyborg;\nimpl Worker for Cyborg {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    // Inherent impl Robot has 0 supertypes
    let req = json!({
        "jsonrpc": "2.0", "id": 716, "method": "typeHierarchy/supertypes",
        "params": {
            "item": { "name": "Robot", "kind": 23, "uri": to_uri(&file) }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 0);

    // Positive control on Cyborg
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 7160, "method": "typeHierarchy/supertypes",
        "params": {
            "item": { "name": "Cyborg", "kind": 23, "uri": to_uri(&file) }
        }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_items = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_items.len(), 1);
    assert_eq!(ctrl_items[0]["name"], "Worker");
}

#[test]
fn test_type_hierarchy_supertypes_successive_isolated_requests() {
    let id = SUPER_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_iso_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("iso_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Trait, visibility: 1, is_macro_expansion: false,
        name: "Greeter".to_string(), path: "iso_crate::Greeter".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 18,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Trait, visibility: 1, is_macro_expansion: false,
        name: "Meower".to_string(), path: "iso_crate::Meower".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 2, start_col: 1, end_line: 2, end_col: 17,
    });
    writer.add_definition(RawDefinition {
        def_id: 3, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "Robot".to_string(), path: "iso_crate::Robot".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 4, start_col: 1, end_line: 4, end_col: 17,
    });
    writer.add_definition(RawDefinition {
        def_id: 4, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "Cat".to_string(), path: "iso_crate::Cat".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 5, start_col: 1, end_line: 5, end_col: 15,
    });
    writer.add_definition(RawDefinition {
        def_id: 5, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<Robot as Greeter>".to_string(), path: "<Robot as Greeter>".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 7, start_col: 1, end_line: 7, end_col: 25,
    });
    writer.add_definition(RawDefinition {
        def_id: 6, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<Cat as Meower>".to_string(), path: "<Cat as Meower>".to_string(),
        signature: None, file: lib_str,
        start_line: 8, start_col: 1, end_line: 8, end_col: 22,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "trait Greeter {}\ntrait Meower {}\n\nstruct Robot;\nstruct Cat;\n\nimpl Greeter for Robot {}\nimpl Meower for Cat {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req1 = json!({
        "jsonrpc": "2.0", "id": 717, "method": "typeHierarchy/supertypes",
        "params": {
            "item": { "name": "Robot", "kind": 23, "uri": to_uri(&file) }
        }
    });
    assert!(dsp.dispatch(&req1));
    let resp1 = dsp.parse_response();
    let items1 = resp1["result"].as_array().unwrap();
    assert_eq!(items1.len(), 1);
    assert_eq!(items1[0]["name"], "Greeter");

    let req2 = json!({
        "jsonrpc": "2.0", "id": 718, "method": "typeHierarchy/supertypes",
        "params": {
            "item": { "name": "Cat", "kind": 23, "uri": to_uri(&file) }
        }
    });
    assert!(dsp.dispatch(&req2));
    let resp2 = dsp.parse_response();
    let items2 = resp2["result"].as_array().unwrap();
    assert_eq!(items2.len(), 1);
    assert_eq!(items2[0]["name"], "Meower");
}

#[test]
fn test_type_hierarchy_supertypes_unregistered_trait_returns_fallback_range() {
    let id = SUPER_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_unreg_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("unreg_crate", "0.1.0");
    // Only index Robot and Impl, do NOT index UnregisteredTrait definition
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "Robot".to_string(), path: "unreg_crate::Robot".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 3, start_col: 1, end_line: 3, end_col: 17,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<Robot as UnregisteredTrait>".to_string(), path: "<Robot as UnregisteredTrait>".to_string(),
        signature: None, file: lib_str,
        start_line: 5, start_col: 1, end_line: 5, end_col: 35,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "trait UnregisteredTrait {}\n\nstruct Robot;\n\nimpl UnregisteredTrait for Robot {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 719, "method": "typeHierarchy/supertypes",
        "params": {
            "item": { "name": "Robot", "kind": 23, "uri": to_uri(&file) }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "UnregisteredTrait");
    assert_eq!(items[0]["kind"], 11);
}
