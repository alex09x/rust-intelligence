/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for typeHierarchy/subtypes (Domain 6, Part 2).
//! Tests deduplication across blocks, symbol kind correctness, references, fallback paths, and numeric IDs.

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

static SUB_P2_COUNTER: AtomicU64 = AtomicU64::new(100);

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
    let id = SUB_P2_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_sub_p2_{}_{}_{}", name, std::process::id(), id));
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
fn test_type_hierarchy_subtypes_deduplication_across_blocks() {
    let id = SUB_P2_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_sub_dedup_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("dedup_sub_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Trait, visibility: 1, is_macro_expansion: false,
        name: "Greeter".to_string(), path: "dedup_sub_crate::Greeter".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 18,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "Robot".to_string(), path: "dedup_sub_crate::Robot".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 3, start_col: 1, end_line: 3, end_col: 17,
    });
    writer.add_definition(RawDefinition {
        def_id: 3, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<Robot as Greeter>".to_string(), path: "<Robot as Greeter>".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 5, start_col: 1, end_line: 5, end_col: 25,
    });
    writer.add_definition(RawDefinition {
        def_id: 4, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<Robot as Greeter>".to_string(), path: "<Robot as Greeter>".to_string(),
        signature: None, file: lib_str,
        start_line: 7, start_col: 1, end_line: 7, end_col: 25,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "trait Greeter {}\n\nstruct Robot;\n\nimpl Greeter for Robot {}\n\nimpl Greeter for Robot {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 736, "method": "typeHierarchy/subtypes",
        "params": {
            "item": { "name": "Greeter", "kind": 11, "uri": to_uri(&file) }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Robot");
}

#[test]
fn test_type_hierarchy_subtypes_symbol_kind_correctness() {
    let id = SUB_P2_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_sub_kinds_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("kinds_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Trait, visibility: 1, is_macro_expansion: false,
        name: "Marker".to_string(), path: "kinds_crate::Marker".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 18,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "SItem".to_string(), path: "kinds_crate::SItem".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 3, start_col: 1, end_line: 3, end_col: 17,
    });
    writer.add_definition(RawDefinition {
        def_id: 3, kind: DefKind::Enum, visibility: 1, is_macro_expansion: false,
        name: "EItem".to_string(), path: "kinds_crate::EItem".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 4, start_col: 1, end_line: 4, end_col: 17,
    });
    writer.add_definition(RawDefinition {
        def_id: 4, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<SItem as Marker>".to_string(), path: "<SItem as Marker>".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 6, start_col: 1, end_line: 6, end_col: 25,
    });
    writer.add_definition(RawDefinition {
        def_id: 5, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<EItem as Marker>".to_string(), path: "<EItem as Marker>".to_string(),
        signature: None, file: lib_str,
        start_line: 7, start_col: 1, end_line: 7, end_col: 25,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "trait Marker {}\n\nstruct SItem;\nenum EItem { A }\n\nimpl Marker for SItem {}\nimpl Marker for EItem {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 737, "method": "typeHierarchy/subtypes",
        "params": {
            "item": { "name": "Marker", "kind": 11, "uri": to_uri(&file) }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["name"], "SItem");
    assert_eq!(items[0]["kind"], 23); // SymbolKind::Struct
    assert_eq!(items[1]["name"], "EItem");
    assert_eq!(items[1]["kind"], 10); // SymbolKind::Enum
}

#[test]
fn test_type_hierarchy_subtypes_reference_based_impl_resolution() {
    let id = SUB_P2_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_sub_ref_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("ref_sub_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Trait, visibility: 1, is_macro_expansion: false,
        name: "Greeter".to_string(), path: "ref_sub_crate::Greeter".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 18,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "Robot".to_string(), path: "ref_sub_crate::Robot".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 3, start_col: 1, end_line: 3, end_col: 17,
    });
    // Add reference on Greeter at line 5 (in `impl Greeter for Robot`)
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 5, start_col: 6, end_line: 5, end_col: 13,
        target_def_id: Some(1), target_path: "ref_sub_crate::Greeter".to_string(),
        target_file: Some(lib_str),
        target_unit: None, target_line: 1, target_col: 1,
        kind: DefKind::Trait, is_write: false, is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "trait Greeter {}\n\nstruct Robot;\n\nimpl Greeter for Robot {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 738, "method": "typeHierarchy/subtypes",
        "params": {
            "item": { "name": "Greeter", "kind": 11, "uri": to_uri(&file) }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Robot");
}

#[test]
fn test_type_hierarchy_subtypes_fallback_path_when_file_read_fails() {
    let id = SUB_P2_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_sub_fb_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let nonexistent_file = dir.join("src/nonexistent.rs");
    let lib_str = nonexistent_file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("fb_sub_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Trait, visibility: 1, is_macro_expansion: false,
        name: "Greeter".to_string(), path: "fb_sub_crate::Greeter".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 18,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "Robot".to_string(), path: "fb_sub_crate::Robot".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 3, start_col: 1, end_line: 3, end_col: 17,
    });
    writer.add_definition(RawDefinition {
        def_id: 3, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<Robot as Greeter>".to_string(), path: "<Robot as Greeter>".to_string(),
        signature: None, file: lib_str,
        start_line: 5, start_col: 1, end_line: 7, end_col: 2,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    // Do NOT create file on disk — fallback parsing extracts Robot from path
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 739, "method": "typeHierarchy/subtypes",
        "params": {
            "item": { "name": "Greeter", "kind": 11, "uri": to_uri(&nonexistent_file) }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Robot");
}

#[test]
fn test_type_hierarchy_subtypes_dispatcher_echoes_numeric_id() {
    let ts = create_temp_shard("num_id");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "trait Greeter {}\n\nstruct Robot;\n\nimpl Greeter for Robot {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 999888, "method": "typeHierarchy/subtypes",
        "params": {
            "item": { "name": "Greeter", "kind": 11, "uri": to_uri(&file) }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 999888);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Robot");
}
