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

#[allow(clippy::too_many_arguments, dead_code)]
fn rdef(
    id: u32,
    kind: DefKind,
    vis: u8,
    mac: bool,
    name: impl Into<String>,
    path: impl Into<String>,
    sig: Option<String>,
    file: impl Into<String>,
    sl: u32,
    sc: u32,
    el: u32,
    ec: u32,
) -> RawDefinition {
    RawDefinition {
        def_id: id,
        kind,
        visibility: vis,
        is_macro_expansion: mac,
        name: name.into(),
        path: path.into(),
        signature: sig,
        file: file.into(),
        start_line: sl,
        start_col: sc,
        end_line: el,
        end_col: ec,
    }
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
fn test_type_hierarchy_supertypes_where_clause_syntax() {
    let id = SUPER_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_where_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("where_crate", "0.1.0");
    writer.add_definition(rdef(1, DefKind::Trait, 1, false, "Serializable", "where_crate::Serializable", None, &lib_str, 1, 1, 1, 25));
    writer.add_definition(rdef(2, DefKind::Struct, 1, false, "Boxed", "where_crate::Boxed", None, &lib_str, 3, 1, 3, 20));
    writer.add_definition(rdef(3, DefKind::Impl, 1, false, "<Boxed as Serializable>", "<Boxed as Serializable>", None, lib_str, 5, 1, 7, 2));
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
    writer.add_definition(rdef(1, DefKind::Trait, 1, false, "Greeter", "qp_crate::Greeter", None, &lib_str, 1, 1, 1, 18));
    writer.add_definition(rdef(2, DefKind::Struct, 1, false, "Robot", "qp_crate::Robot", None, &lib_str, 3, 1, 3, 17));
    writer.add_definition(rdef(3, DefKind::Impl, 1, false, "<Robot as Greeter>", "<Robot as Greeter>", None, lib_str, 5, 1, 5, 40));
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
    writer.add_definition(rdef(1, DefKind::Struct, 1, false, "Robot", "inh_crate::Robot", None, &lib_str, 1, 1, 1, 17));
    writer.add_definition(rdef(2, DefKind::Impl, 1, false, "Robot", "inh_crate::Robot", None, &lib_str, 3, 1, 5, 2));
    writer.add_definition(rdef(3, DefKind::Trait, 1, false, "Worker", "inh_crate::Worker", None, &lib_str, 7, 1, 7, 17));
    writer.add_definition(rdef(4, DefKind::Struct, 1, false, "Cyborg", "inh_crate::Cyborg", None, &lib_str, 9, 1, 9, 18));
    writer.add_definition(rdef(5, DefKind::Impl, 1, false, "<Cyborg as Worker>", "<Cyborg as Worker>", None, lib_str, 11, 1, 11, 30));
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
    writer.add_definition(rdef(1, DefKind::Trait, 1, false, "Greeter", "iso_crate::Greeter", None, &lib_str, 1, 1, 1, 18));
    writer.add_definition(rdef(2, DefKind::Trait, 1, false, "Meower", "iso_crate::Meower", None, &lib_str, 2, 1, 2, 17));
    writer.add_definition(rdef(3, DefKind::Struct, 1, false, "Robot", "iso_crate::Robot", None, &lib_str, 4, 1, 4, 17));
    writer.add_definition(rdef(4, DefKind::Struct, 1, false, "Cat", "iso_crate::Cat", None, &lib_str, 5, 1, 5, 15));
    writer.add_definition(rdef(5, DefKind::Impl, 1, false, "<Robot as Greeter>", "<Robot as Greeter>", None, &lib_str, 7, 1, 7, 25));
    writer.add_definition(rdef(6, DefKind::Impl, 1, false, "<Cat as Meower>", "<Cat as Meower>", None, lib_str, 8, 1, 8, 22));
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
    writer.add_definition(rdef(1, DefKind::Struct, 1, false, "Robot", "unreg_crate::Robot", None, &lib_str, 3, 1, 3, 17));
    writer.add_definition(rdef(2, DefKind::Impl, 1, false, "<Robot as UnregisteredTrait>", "<Robot as UnregisteredTrait>", None, lib_str, 5, 1, 5, 35));
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
