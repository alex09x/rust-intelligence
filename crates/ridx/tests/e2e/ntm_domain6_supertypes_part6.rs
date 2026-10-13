/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for typeHierarchy/supertypes (Domain 6, Part 6).
//! Tests cross-file impls, cross-file traits, qualified paths, and empty blocks.

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

static SUPER_P6_COUNTER: AtomicU64 = AtomicU64::new(600);

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
fn test_type_hierarchy_supertypes_cross_file_impl() {
    let id = SUPER_P6_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_p6_cfimpl_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let lib_file = dir.join("src/lib.rs");
    let impl_file = dir.join("src/impls.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();
    let impl_str = impl_file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("cfimpl_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Trait, visibility: 1, is_macro_expansion: false,
        name: "RemoteTrait".to_string(), path: "cfimpl_crate::RemoteTrait".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 21,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "RemoteStruct".to_string(), path: "cfimpl_crate::RemoteStruct".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 2, start_col: 1, end_line: 2, end_col: 22,
    });
    writer.add_definition(RawDefinition {
        def_id: 3, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<RemoteStruct as RemoteTrait>".to_string(), path: "<RemoteStruct as RemoteTrait>".to_string(),
        signature: None, file: impl_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 38,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    std::fs::write(&lib_file, "trait RemoteTrait {}\nstruct RemoteStruct;\n").unwrap();
    std::fs::write(&impl_file, "impl RemoteTrait for RemoteStruct {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 806, "method": "typeHierarchy/supertypes",
        "params": { "item": { "name": "RemoteStruct", "kind": 23, "uri": to_uri(&lib_file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 806);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "RemoteTrait");
}

#[test]
fn test_type_hierarchy_supertypes_cross_file_trait_definition() {
    let id = SUPER_P6_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_p6_cftrait_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let lib_file = dir.join("src/lib.rs");
    let trait_file = dir.join("src/traits.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();
    let trait_str = trait_file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("cftrait_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Trait, visibility: 1, is_macro_expansion: false,
        name: "ExternalTrait".to_string(), path: "cftrait_crate::ExternalTrait".to_string(),
        signature: None, file: trait_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 23,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "LocalType".to_string(), path: "cftrait_crate::LocalType".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 19,
    });
    writer.add_definition(RawDefinition {
        def_id: 3, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<LocalType as ExternalTrait>".to_string(), path: "<LocalType as ExternalTrait>".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 3, start_col: 1, end_line: 3, end_col: 37,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    std::fs::write(&trait_file, "pub trait ExternalTrait {}\n").unwrap();
    std::fs::write(&lib_file, "struct LocalType;\n\nimpl ExternalTrait for LocalType {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 807, "method": "typeHierarchy/supertypes",
        "params": { "item": { "name": "LocalType", "kind": 23, "uri": to_uri(&lib_file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 807);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "ExternalTrait");
    assert_eq!(items[0]["uri"], to_uri(&trait_file));
}

#[test]
fn test_type_hierarchy_supertypes_qualified_struct_in_impl() {
    let id = SUPER_P6_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_p6_qual_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("qual_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Trait, visibility: 1, is_macro_expansion: false,
        name: "Serialize".to_string(), path: "qual_crate::Serialize".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 19,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "Account".to_string(), path: "qual_crate::models::Account".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 2, start_col: 1, end_line: 2, end_col: 17,
    });
    writer.add_definition(RawDefinition {
        def_id: 3, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<models::Account as Serialize>".to_string(), path: "<models::Account as Serialize>".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 4, start_col: 1, end_line: 4, end_col: 39,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let code = "trait Serialize {}\nstruct Account;\n\nimpl Serialize for models::Account {}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 808, "method": "typeHierarchy/supertypes",
        "params": { "item": { "name": "Account", "kind": 23, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 808);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Serialize");
}

#[test]
fn test_type_hierarchy_supertypes_empty_impl_block() {
    let id = SUPER_P6_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_p6_empty_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("empty_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Trait, visibility: 1, is_macro_expansion: false,
        name: "EmptyTrait".to_string(), path: "empty_crate::EmptyTrait".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 20,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Struct, visibility: 1, is_macro_expansion: false,
        name: "EmptyStruct".to_string(), path: "empty_crate::EmptyStruct".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 2, start_col: 1, end_line: 2, end_col: 21,
    });
    writer.add_definition(RawDefinition {
        def_id: 3, kind: DefKind::Impl, visibility: 1, is_macro_expansion: false,
        name: "<EmptyStruct as EmptyTrait>".to_string(), path: "<EmptyStruct as EmptyTrait>".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 4, start_col: 1, end_line: 4, end_col: 33,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let code = "trait EmptyTrait {}\nstruct EmptyStruct;\n\nimpl EmptyTrait for EmptyStruct {}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 809, "method": "typeHierarchy/supertypes",
        "params": { "item": { "name": "EmptyStruct", "kind": 23, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 809);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "EmptyTrait");
}
