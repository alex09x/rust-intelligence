/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for typeHierarchy/subtypes (Domain 6, Part 10).
//! Tests multi-root workspaces, generic item names, generic struct names, and mixed struct/enum implementors.

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

static SUB_P10_COUNTER: AtomicU64 = AtomicU64::new(1000);

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
fn test_type_hierarchy_subtypes_multiple_workspaces_first_workspace_used() {
    let id = SUB_P10_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir1 = std::env::temp_dir().join(format!("ridx_sub_p10_ws1_{}_{}", std::process::id(), id));
    let dir2 = std::env::temp_dir().join(format!("ridx_sub_p10_ws2_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir1.join("src")).unwrap();
    std::fs::create_dir_all(dir2.join("src")).unwrap();
    let shard1 = dir1.join("index.ridx");
    let shard2 = dir2.join("index.ridx");
    let file1 = dir1.join("src/lib.rs");
    let file1_str = file1.to_str().unwrap().to_string();
    let mut writer1 = IndexWriter::new("ws1_sub_crate", "0.1.0");
    writer1.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 1,
        is_macro_expansion: false,
        name: "FirstWsTrait".to_string(),
        path: "ws1_sub_crate::FirstWsTrait".to_string(),
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
        name: "FirstWsImpl".to_string(),
        path: "ws1_sub_crate::FirstWsImpl".to_string(),
        signature: None,
        file: file1_str.clone(),
        start_line: 2,
        start_col: 1,
        end_line: 2,
        end_col: 21,
    });
    writer1.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Impl,
        visibility: 1,
        is_macro_expansion: false,
        name: "<FirstWsImpl as FirstWsTrait>".to_string(),
        path: "<FirstWsImpl as FirstWsTrait>".to_string(),
        signature: None,
        file: file1_str.clone(),
        start_line: 4,
        start_col: 1,
        end_line: 4,
        end_col: 37,
    });
    writer1.write_to(&mut File::create(&shard1).unwrap()).unwrap();
    std::fs::write(&file1, "trait FirstWsTrait {}\nstruct FirstWsImpl;\n\nimpl FirstWsTrait for FirstWsImpl {}\n").unwrap();
    let mut writer2 = IndexWriter::new("ws2_sub_crate", "0.1.0");
    writer2.write_to(&mut File::create(&shard2).unwrap()).unwrap();
    let mut dsp = TestDispatcher::new_multi(vec![shard1, shard2]);
    let req = json!({
        "jsonrpc": "2.0", "id": 852, "method": "typeHierarchy/subtypes",
        "params": { "item": { "name": "FirstWsTrait", "kind": 11, "uri": to_uri(&file1) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 852);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "FirstWsImpl");
}

#[test]
fn test_type_hierarchy_subtypes_trait_name_with_generics_in_item() {
    let id = SUB_P10_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_sub_p10_genitem_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("genitem_sub_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 1,
        is_macro_expansion: false,
        name: "ParserTrait".to_string(),
        path: "genitem_sub_crate::ParserTrait".to_string(),
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
        name: "JsonParser".to_string(),
        path: "genitem_sub_crate::JsonParser".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 2,
        start_col: 1,
        end_line: 2,
        end_col: 20,
    });
    writer.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Impl,
        visibility: 1,
        is_macro_expansion: false,
        name: "<JsonParser as ParserTrait>".to_string(),
        path: "<JsonParser as ParserTrait>".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 4,
        start_col: 1,
        end_line: 4,
        end_col: 41,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    let code = "trait ParserTrait<T> {}\nstruct JsonParser;\n\nimpl ParserTrait<String> for JsonParser {}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);
    // item.name has generic parameter "ParserTrait<String>" -> base "ParserTrait"
    let req = json!({
        "jsonrpc": "2.0", "id": 853, "method": "typeHierarchy/subtypes",
        "params": { "item": { "name": "ParserTrait<String>", "kind": 11, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 853);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "JsonParser");
}

#[test]
fn test_type_hierarchy_subtypes_struct_name_with_generics_in_result() {
    let id = SUB_P10_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_sub_p10_resgen_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("resgen_sub_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 1,
        is_macro_expansion: false,
        name: "Codec".to_string(),
        path: "resgen_sub_crate::Codec".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 16,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Struct,
        visibility: 1,
        is_macro_expansion: false,
        name: "GenCodec".to_string(),
        path: "resgen_sub_crate::GenCodec".to_string(),
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
        name: "<GenCodec as Codec>".to_string(),
        path: "<GenCodec as Codec>".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 4,
        start_col: 1,
        end_line: 4,
        end_col: 36,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    let code = "trait Codec {}\nstruct GenCodec<T>(T);\n\nimpl<T> Codec for GenCodec<T> {}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 854, "method": "typeHierarchy/subtypes",
        "params": { "item": { "name": "Codec", "kind": 11, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 854);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "GenCodec");
}

#[test]
fn test_type_hierarchy_subtypes_both_struct_and_enum_implementors() {
    let id = SUB_P10_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_sub_p10_mix_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("mix_sub_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 1,
        is_macro_expansion: false,
        name: "MixedTrait".to_string(),
        path: "mix_sub_crate::MixedTrait".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 21,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Struct,
        visibility: 1,
        is_macro_expansion: false,
        name: "StructImpl".to_string(),
        path: "mix_sub_crate::StructImpl".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 2,
        start_col: 1,
        end_line: 2,
        end_col: 20,
    });
    writer.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Enum,
        visibility: 1,
        is_macro_expansion: false,
        name: "EnumImpl".to_string(),
        path: "mix_sub_crate::EnumImpl".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 3,
        start_col: 1,
        end_line: 3,
        end_col: 23,
    });
    writer.add_definition(RawDefinition {
        def_id: 4,
        kind: DefKind::Impl,
        visibility: 1,
        is_macro_expansion: false,
        name: "<StructImpl as MixedTrait>".to_string(),
        path: "<StructImpl as MixedTrait>".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 5,
        start_col: 1,
        end_line: 5,
        end_col: 33,
    });
    writer.add_definition(RawDefinition {
        def_id: 5,
        kind: DefKind::Impl,
        visibility: 1,
        is_macro_expansion: false,
        name: "<EnumImpl as MixedTrait>".to_string(),
        path: "<EnumImpl as MixedTrait>".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 6,
        start_col: 1,
        end_line: 6,
        end_col: 31,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    let code = "trait MixedTrait {}\nstruct StructImpl;\nenum EnumImpl { X, Y }\n\nimpl MixedTrait for StructImpl {}\nimpl MixedTrait for EnumImpl {}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 855, "method": "typeHierarchy/subtypes",
        "params": { "item": { "name": "MixedTrait", "kind": 11, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 855);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    let kinds: Vec<u64> = items.iter().map(|it| it["kind"].as_u64().unwrap()).collect();
    assert_eq!(kinds, vec![23, 10]);
}
