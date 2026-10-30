/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for typeHierarchy/supertypes (Domain 6, Part 10).
//! Tests lifetime bounds, multi-root workspace handling, generic item names, and generic trait base names.

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

static SUPER_P10_COUNTER: AtomicU64 = AtomicU64::new(1000);

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

    fn new_multi(shards: Vec<PathBuf>) -> Self {
        let mut d = Self {
            lifecycle: LifecycleManager::new(),
            multi_root: MultiRootWorkspace::new(),
            overlay_mgr: OverlayManager::new(),
            cancelled: Arc::new(Mutex::new(HashSet::new())),
            writer: Vec::new(),
        };
        for s in shards {
            assert!(d.multi_root.add_root(s));
        }
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
fn test_type_hierarchy_supertypes_trait_with_lifetime_bounds() {
    let id = SUPER_P10_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_p10_lt_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("lt_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 1,
        is_macro_expansion: false,
        name: "BorrowedTrait".to_string(),
        path: "lt_crate::BorrowedTrait".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 23,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Struct,
        visibility: 1,
        is_macro_expansion: false,
        name: "RefHolder".to_string(),
        path: "lt_crate::RefHolder".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 2,
        start_col: 1,
        end_line: 2,
        end_col: 24,
    });
    writer.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Impl,
        visibility: 1,
        is_macro_expansion: false,
        name: "<RefHolder as BorrowedTrait>".to_string(),
        path: "<RefHolder as BorrowedTrait>".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 4,
        start_col: 1,
        end_line: 4,
        end_col: 51,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let code = "trait BorrowedTrait<'a> {}\nstruct RefHolder<'a>(&'a str);\n\nimpl<'a> BorrowedTrait<'a> for RefHolder<'a> {}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 823, "method": "typeHierarchy/supertypes",
        "params": { "item": { "name": "RefHolder", "kind": 23, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 823);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "BorrowedTrait");
}

#[test]
fn test_type_hierarchy_supertypes_multiple_workspaces_first_workspace_used() {
    let id = SUPER_P10_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir1 = std::env::temp_dir().join(format!("ridx_super_p10_ws1_{}_{}", std::process::id(), id));
    let dir2 = std::env::temp_dir().join(format!("ridx_super_p10_ws2_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir1.join("src")).unwrap();
    std::fs::create_dir_all(dir2.join("src")).unwrap();
    let shard1 = dir1.join("index.ridx");
    let shard2 = dir2.join("index.ridx");
    let file1 = dir1.join("src/lib.rs");
    let file1_str = file1.to_str().unwrap().to_string();

    let mut writer1 = IndexWriter::new("ws1_crate", "0.1.0");
    writer1.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 1,
        is_macro_expansion: false,
        name: "FirstWsTrait".to_string(),
        path: "ws1_crate::FirstWsTrait".to_string(),
        signature: None,
        file: file1_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 22,
    });
    writer1.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Struct,
        visibility: 1,
        is_macro_expansion: false,
        name: "FirstWsStruct".to_string(),
        path: "ws1_crate::FirstWsStruct".to_string(),
        signature: None,
        file: file1_str.clone(),
        start_line: 2,
        start_col: 1,
        end_line: 2,
        end_col: 23,
    });
    writer1.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Impl,
        visibility: 1,
        is_macro_expansion: false,
        name: "<FirstWsStruct as FirstWsTrait>".to_string(),
        path: "<FirstWsStruct as FirstWsTrait>".to_string(),
        signature: None,
        file: file1_str.clone(),
        start_line: 4,
        start_col: 1,
        end_line: 4,
        end_col: 39,
    });
    writer1.write_to(&mut File::create(&shard1).unwrap()).unwrap();
    std::fs::write(&file1, "trait FirstWsTrait {}\nstruct FirstWsStruct;\n\nimpl FirstWsTrait for FirstWsStruct {}\n").unwrap();

    let mut writer2 = IndexWriter::new("ws2_crate", "0.1.0");
    writer2.write_to(&mut File::create(&shard2).unwrap()).unwrap();

    let mut dsp = TestDispatcher::new_multi(vec![shard1, shard2]);

    let req = json!({
        "jsonrpc": "2.0", "id": 824, "method": "typeHierarchy/supertypes",
        "params": { "item": { "name": "FirstWsStruct", "kind": 23, "uri": to_uri(&file1) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 824);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "FirstWsTrait");
}

#[test]
fn test_type_hierarchy_supertypes_struct_name_with_generics_in_item() {
    let id = SUPER_P10_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_p10_genitem_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("genitem_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 1,
        is_macro_expansion: false,
        name: "Identifiable".to_string(),
        path: "genitem_crate::Identifiable".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 22,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Struct,
        visibility: 1,
        is_macro_expansion: false,
        name: "Entity".to_string(),
        path: "genitem_crate::Entity".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 2,
        start_col: 1,
        end_line: 2,
        end_col: 22,
    });
    writer.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Impl,
        visibility: 1,
        is_macro_expansion: false,
        name: "<Entity as Identifiable>".to_string(),
        path: "<Entity as Identifiable>".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 4,
        start_col: 1,
        end_line: 4,
        end_col: 36,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let code = "trait Identifiable {}\nstruct Entity<ID>(ID);\n\nimpl<ID> Identifiable for Entity<ID> {}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    // item.name is "Entity<u64>" - should match base "Entity"
    let req = json!({
        "jsonrpc": "2.0", "id": 825, "method": "typeHierarchy/supertypes",
        "params": { "item": { "name": "Entity<u64>", "kind": 23, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 825);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Identifiable");
}

#[test]
fn test_type_hierarchy_supertypes_trait_name_with_generics_in_result() {
    let id = SUPER_P10_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_p10_resgen_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("resgen_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 1,
        is_macro_expansion: false,
        name: "Converter".to_string(),
        path: "resgen_crate::Converter".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 19,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Struct,
        visibility: 1,
        is_macro_expansion: false,
        name: "Parser".to_string(),
        path: "resgen_crate::Parser".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 2,
        start_col: 1,
        end_line: 2,
        end_col: 16,
    });
    writer.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Impl,
        visibility: 1,
        is_macro_expansion: false,
        name: "<Parser as Converter>".to_string(),
        path: "<Parser as Converter>".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 4,
        start_col: 1,
        end_line: 4,
        end_col: 36,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let code = "trait Converter<T> {}\nstruct Parser;\n\nimpl Converter<String> for Parser {}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 826, "method": "typeHierarchy/supertypes",
        "params": { "item": { "name": "Parser", "kind": 23, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 826);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    // Result trait name should be base name "Converter"
    assert_eq!(items[0]["name"], "Converter");
}
