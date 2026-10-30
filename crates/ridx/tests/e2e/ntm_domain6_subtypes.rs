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

fn create_temp_shard(name: &str) -> TempShard {
    let id = SUB_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_sub_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let lib_file = dir.join("src/lib.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new(name, "0.1.0");
    writer.add_definition(rdef(1, DefKind::Trait, 1, false, "Greeter", format!("{}::Greeter", name), Some("pub trait Greeter".to_string()), &lib_str, 1, 1, 1, 18));
    writer.add_definition(rdef(2, DefKind::Struct, 1, false, "Robot", format!("{}::Robot", name), Some("pub struct Robot".to_string()), &lib_str, 3, 1, 3, 17));
    writer.add_definition(rdef(3, DefKind::Impl, 1, false, "<Robot as Greeter>", "<Robot as Greeter>", Some("impl Greeter for Robot".to_string()), lib_str, 5, 1, 7, 2));
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
    writer.add_definition(rdef(1, DefKind::Trait, 1, false, "Greeter", "multi_sub_crate::Greeter", None, &lib_str, 1, 1, 1, 18));
    writer.add_definition(rdef(2, DefKind::Struct, 1, false, "Robot", "multi_sub_crate::Robot", None, &lib_str, 3, 1, 3, 17));
    writer.add_definition(rdef(3, DefKind::Struct, 1, false, "Human", "multi_sub_crate::Human", None, &lib_str, 4, 1, 4, 17));
    writer.add_definition(rdef(4, DefKind::Impl, 1, false, "<Robot as Greeter>", "<Robot as Greeter>", None, &lib_str, 6, 1, 6, 25));
    writer.add_definition(rdef(5, DefKind::Impl, 1, false, "<Human as Greeter>", "<Human as Greeter>", None, lib_str, 7, 1, 7, 25));
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
    writer.add_definition(rdef(1, DefKind::Trait, 1, false, "Describable", "enum_sub_crate::Describable", None, &lib_str, 1, 1, 1, 22));
    writer.add_definition(rdef(2, DefKind::Enum, 1, false, "Status", "enum_sub_crate::Status", None, &lib_str, 3, 1, 3, 18));
    writer.add_definition(rdef(3, DefKind::Impl, 1, false, "<Status as Describable>", "<Status as Describable>", None, lib_str, 5, 1, 5, 35));
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
    writer.add_definition(rdef(1, DefKind::Trait, 1, false, "Container", "gs_sub_crate::Container", None, &lib_str, 1, 1, 1, 20));
    writer.add_definition(rdef(2, DefKind::Struct, 1, false, "Wrapper", "gs_sub_crate::Wrapper", None, &lib_str, 3, 1, 3, 20));
    writer.add_definition(rdef(3, DefKind::Impl, 1, false, "<Wrapper as Container>", "<Wrapper as Container>", None, lib_str, 5, 1, 5, 40));
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
