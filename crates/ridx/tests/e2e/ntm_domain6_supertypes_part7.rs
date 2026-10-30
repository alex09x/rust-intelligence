/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for typeHierarchy/supertypes (Domain 6, Part 7).
//! Tests macro expansion flags, const generic params, nested types, and exact item schema.

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

static SUPER_P7_COUNTER: AtomicU64 = AtomicU64::new(700);

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
fn test_type_hierarchy_supertypes_macro_generated_impl_flag() {
    let id = SUPER_P7_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_p7_macro_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("macro_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 1,
        is_macro_expansion: false,
        name: "DeriveTrait".to_string(),
        path: "macro_crate::DeriveTrait".to_string(),
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
        name: "DeriveTarget".to_string(),
        path: "macro_crate::DeriveTarget".to_string(),
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
        is_macro_expansion: true,
        name: "<DeriveTarget as DeriveTrait>".to_string(),
        path: "<DeriveTarget as DeriveTrait>".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 4,
        start_col: 1,
        end_line: 4,
        end_col: 36,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let code = "trait DeriveTrait {}\nstruct DeriveTarget;\n\nimpl DeriveTrait for DeriveTarget {}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 810, "method": "typeHierarchy/supertypes",
        "params": { "item": { "name": "DeriveTarget", "kind": 23, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 810);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "DeriveTrait");
}

#[test]
fn test_type_hierarchy_supertypes_generic_const_param_impl() {
    let id = SUPER_P7_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_p7_constgen_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("constgen_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 1,
        is_macro_expansion: false,
        name: "FixedSize".to_string(),
        path: "constgen_crate::FixedSize".to_string(),
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
        name: "FixedBuffer".to_string(),
        path: "constgen_crate::FixedBuffer".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 2,
        start_col: 1,
        end_line: 2,
        end_col: 31,
    });
    writer.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Impl,
        visibility: 1,
        is_macro_expansion: false,
        name: "<FixedBuffer as FixedSize>".to_string(),
        path: "<FixedBuffer as FixedSize>".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 4,
        start_col: 1,
        end_line: 4,
        end_col: 49,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let code = "trait FixedSize {}\nstruct FixedBuffer<const N: usize>;\n\nimpl<const N: usize> FixedSize for FixedBuffer<N> {}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 811, "method": "typeHierarchy/supertypes",
        "params": { "item": { "name": "FixedBuffer", "kind": 23, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 811);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "FixedSize");
}

#[test]
fn test_type_hierarchy_supertypes_nested_type_parameters() {
    let id = SUPER_P7_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_p7_nest_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("nest_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 1,
        is_macro_expansion: false,
        name: "NestedTrait".to_string(),
        path: "nest_crate::NestedTrait".to_string(),
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
        name: "Container".to_string(),
        path: "nest_crate::Container".to_string(),
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
        name: "<Container as NestedTrait>".to_string(),
        path: "<Container as NestedTrait>".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 4,
        start_col: 1,
        end_line: 4,
        end_col: 47,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let code = "trait NestedTrait {}\nstruct Container<T>(T);\n\nimpl NestedTrait for Container<Vec<String>> {}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 812, "method": "typeHierarchy/supertypes",
        "params": { "item": { "name": "Container", "kind": 23, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 812);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "NestedTrait");
}

#[test]
fn test_type_hierarchy_supertypes_supertypes_item_shape_exact_fields() {
    let id = SUPER_P7_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_p7_shape_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("shape_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 1,
        is_macro_expansion: false,
        name: "ShapeTrait".to_string(),
        path: "shape_crate::ShapeTrait".to_string(),
        signature: Some("pub trait ShapeTrait".to_string()),
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 20,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Struct,
        visibility: 1,
        is_macro_expansion: false,
        name: "ShapeTarget".to_string(),
        path: "shape_crate::ShapeTarget".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 2,
        start_col: 1,
        end_line: 2,
        end_col: 21,
    });
    writer.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Impl,
        visibility: 1,
        is_macro_expansion: false,
        name: "<ShapeTarget as ShapeTrait>".to_string(),
        path: "<ShapeTarget as ShapeTrait>".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 4,
        start_col: 1,
        end_line: 4,
        end_col: 33,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let code = "trait ShapeTrait {}\nstruct ShapeTarget;\n\nimpl ShapeTrait for ShapeTarget {}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 813, "method": "typeHierarchy/supertypes",
        "params": { "item": { "name": "ShapeTarget", "kind": 23, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 813);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    let trait_item = &items[0];

    assert_eq!(trait_item["name"], "ShapeTrait");
    assert_eq!(trait_item["kind"], 11);
    assert_eq!(trait_item["uri"], to_uri(&file));
    assert_eq!(trait_item["range"]["start"]["line"], 0);
    assert_eq!(trait_item["range"]["start"]["character"], 0);
    assert_eq!(trait_item["range"]["end"]["line"], 0);
    assert_eq!(trait_item["range"]["end"]["character"], 19);
    assert_eq!(trait_item["selectionRange"], trait_item["range"]);
}
