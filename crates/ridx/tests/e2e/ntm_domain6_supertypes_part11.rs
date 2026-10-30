/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for typeHierarchy/supertypes (Domain 6, Part 11).
//! Tests multiple impls ordering, extra whitespace formatting, inherent impl filtering, and roundtrip JSON-RPC structure.

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

static SUPER_P11_COUNTER: AtomicU64 = AtomicU64::new(1100);

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
fn test_type_hierarchy_supertypes_multiple_impls_different_traits_ordered() {
    let id = SUPER_P11_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_p11_order_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("order_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 1,
        is_macro_expansion: false,
        name: "AlphaTrait".to_string(),
        path: "order_crate::AlphaTrait".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 20,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Trait,
        visibility: 1,
        is_macro_expansion: false,
        name: "BetaTrait".to_string(),
        path: "order_crate::BetaTrait".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 2,
        start_col: 1,
        end_line: 2,
        end_col: 19,
    });
    writer.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Struct,
        visibility: 1,
        is_macro_expansion: false,
        name: "DualType".to_string(),
        path: "order_crate::DualType".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 3,
        start_col: 1,
        end_line: 3,
        end_col: 18,
    });
    writer.add_definition(RawDefinition {
        def_id: 4,
        kind: DefKind::Impl,
        visibility: 1,
        is_macro_expansion: false,
        name: "<DualType as AlphaTrait>".to_string(),
        path: "<DualType as AlphaTrait>".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 5,
        start_col: 1,
        end_line: 5,
        end_col: 30,
    });
    writer.add_definition(RawDefinition {
        def_id: 5,
        kind: DefKind::Impl,
        visibility: 1,
        is_macro_expansion: false,
        name: "<DualType as BetaTrait>".to_string(),
        path: "<DualType as BetaTrait>".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 6,
        start_col: 1,
        end_line: 6,
        end_col: 29,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let code = "trait AlphaTrait {}\ntrait BetaTrait {}\nstruct DualType;\n\nimpl AlphaTrait for DualType {}\nimpl BetaTrait for DualType {}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 827, "method": "typeHierarchy/supertypes",
        "params": { "item": { "name": "DualType", "kind": 23, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 827);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 2);
}

#[test]
fn test_type_hierarchy_supertypes_whitespace_around_names_in_impl() {
    let id = SUPER_P11_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_p11_space_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("space_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 1,
        is_macro_expansion: false,
        name: "SpacedTrait".to_string(),
        path: "space_crate::SpacedTrait".to_string(),
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
        name: "SpacedStruct".to_string(),
        path: "space_crate::SpacedStruct".to_string(),
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
        name: "<SpacedStruct as SpacedTrait>".to_string(),
        path: "<SpacedStruct as SpacedTrait>".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 4,
        start_col: 1,
        end_line: 4,
        end_col: 45,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let code = "trait SpacedTrait {}\nstruct SpacedStruct;\n\nimpl   SpacedTrait   for   SpacedStruct   {}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 828, "method": "typeHierarchy/supertypes",
        "params": { "item": { "name": "SpacedStruct", "kind": 23, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 828);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "SpacedTrait");
}

#[test]
fn test_type_hierarchy_supertypes_inherent_impl_block_ignored() {
    let id = SUPER_P11_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_p11_inherent_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("inherent_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 1,
        is_macro_expansion: false,
        name: "OnlyInherent".to_string(),
        path: "inherent_crate::OnlyInherent".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 22,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Impl,
        visibility: 1,
        is_macro_expansion: false,
        name: "OnlyInherent".to_string(),
        path: "OnlyInherent".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 3,
        start_col: 1,
        end_line: 5,
        end_col: 2,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let code = "struct OnlyInherent;\n\nimpl OnlyInherent {\n    pub fn helper() {}\n}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 829, "method": "typeHierarchy/supertypes",
        "params": { "item": { "name": "OnlyInherent", "kind": 23, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 829);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_type_hierarchy_supertypes_roundtrip_response_structure() {
    let id = SUPER_P11_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_p11_roundtrip_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("rt_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 1,
        is_macro_expansion: false,
        name: "RoundTrait".to_string(),
        path: "rt_crate::RoundTrait".to_string(),
        signature: None,
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
        name: "RoundStruct".to_string(),
        path: "rt_crate::RoundStruct".to_string(),
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
        name: "<RoundStruct as RoundTrait>".to_string(),
        path: "<RoundStruct as RoundTrait>".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 4,
        start_col: 1,
        end_line: 4,
        end_col: 33,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let code = "trait RoundTrait {}\nstruct RoundStruct;\n\nimpl RoundTrait for RoundStruct {}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 830, "method": "typeHierarchy/supertypes",
        "params": {
            "item": {
                "name": "RoundStruct", "kind": 23, "uri": to_uri(&file),
                "range": { "start": { "line": 1, "character": 0 }, "end": { "line": 1, "character": 19 } },
                "selectionRange": { "start": { "line": 1, "character": 7 }, "end": { "line": 1, "character": 18 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 830);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "RoundTrait");
    assert_eq!(items[0]["kind"], 11);
    assert_eq!(items[0]["uri"], to_uri(&file));
}
