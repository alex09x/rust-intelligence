/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for typeHierarchy/supertypes (Domain 6, Part 9).
//! Tests reference-based enum supertypes, non-impl references, consecutive queries, and role inversion rejection.

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

static SUPER_P9_COUNTER: AtomicU64 = AtomicU64::new(900);

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
fn test_type_hierarchy_supertypes_reference_based_enum_supertypes() {
    let id = SUPER_P9_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_p9_refenum_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("refenum_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 1,
        is_macro_expansion: false,
        name: "Action".to_string(),
        path: "refenum_crate::Action".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 16,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Enum,
        visibility: 1,
        is_macro_expansion: false,
        name: "Command".to_string(),
        path: "refenum_crate::Command".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 2,
        start_col: 1,
        end_line: 2,
        end_col: 28,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 4,
        start_col: 18,
        end_line: 4,
        end_col: 25,
        target_def_id: Some(2),
        target_path: "refenum_crate::Command".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 2,
        target_col: 1,
        kind: DefKind::Enum,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let code = "trait Action {}\nenum Command { Run, Stop }\n\nimpl Action for Command {}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 818, "method": "typeHierarchy/supertypes",
        "params": { "item": { "name": "Command", "kind": 10, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 818);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Action");
}

#[test]
fn test_type_hierarchy_supertypes_reference_without_matching_impl_line_ignored() {
    let id = SUPER_P9_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_p9_nonimplref_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("nonimplref_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 1,
        is_macro_expansion: false,
        name: "Standalone".to_string(),
        path: "nonimplref_crate::Standalone".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 21,
    });
    // Reference inside a function (not an impl line)
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 4,
        start_col: 14,
        end_line: 4,
        end_col: 24,
        target_def_id: Some(1),
        target_path: "nonimplref_crate::Standalone".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let code = "struct Standalone;\n\nfn use_it() {\n    let _x: Standalone;\n}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 819, "method": "typeHierarchy/supertypes",
        "params": { "item": { "name": "Standalone", "kind": 23, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 819);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_type_hierarchy_supertypes_consecutive_requests_different_structs() {
    let id = SUPER_P9_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_p9_consec_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("consec_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 1,
        is_macro_expansion: false,
        name: "TraitOne".to_string(),
        path: "consec_crate::TraitOne".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 18,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Trait,
        visibility: 1,
        is_macro_expansion: false,
        name: "TraitTwo".to_string(),
        path: "consec_crate::TraitTwo".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 2,
        start_col: 1,
        end_line: 2,
        end_col: 18,
    });
    writer.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Struct,
        visibility: 1,
        is_macro_expansion: false,
        name: "TypeOne".to_string(),
        path: "consec_crate::TypeOne".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 3,
        start_col: 1,
        end_line: 3,
        end_col: 17,
    });
    writer.add_definition(RawDefinition {
        def_id: 4,
        kind: DefKind::Struct,
        visibility: 1,
        is_macro_expansion: false,
        name: "TypeTwo".to_string(),
        path: "consec_crate::TypeTwo".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 4,
        start_col: 1,
        end_line: 4,
        end_col: 17,
    });
    writer.add_definition(RawDefinition {
        def_id: 5,
        kind: DefKind::Impl,
        visibility: 1,
        is_macro_expansion: false,
        name: "<TypeOne as TraitOne>".to_string(),
        path: "<TypeOne as TraitOne>".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 6,
        start_col: 1,
        end_line: 6,
        end_col: 27,
    });
    writer.add_definition(RawDefinition {
        def_id: 6,
        kind: DefKind::Impl,
        visibility: 1,
        is_macro_expansion: false,
        name: "<TypeTwo as TraitTwo>".to_string(),
        path: "<TypeTwo as TraitTwo>".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 7,
        start_col: 1,
        end_line: 7,
        end_col: 27,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let code = "trait TraitOne {}\ntrait TraitTwo {}\nstruct TypeOne;\nstruct TypeTwo;\n\nimpl TraitOne for TypeOne {}\nimpl TraitTwo for TypeTwo {}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    // Request 1 for TypeOne
    let req1 = json!({
        "jsonrpc": "2.0", "id": 820, "method": "typeHierarchy/supertypes",
        "params": { "item": { "name": "TypeOne", "kind": 23, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req1));
    let resp1 = dsp.parse_response();
    assert_eq!(resp1["id"], 820);
    let items1 = resp1["result"].as_array().unwrap();
    assert_eq!(items1.len(), 1);
    assert_eq!(items1[0]["name"], "TraitOne");

    // Request 2 for TypeTwo
    let req2 = json!({
        "jsonrpc": "2.0", "id": 821, "method": "typeHierarchy/supertypes",
        "params": { "item": { "name": "TypeTwo", "kind": 23, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req2));
    let resp2 = dsp.parse_response();
    assert_eq!(resp2["id"], 821);
    let items2 = resp2["result"].as_array().unwrap();
    assert_eq!(items2.len(), 1);
    assert_eq!(items2[0]["name"], "TraitTwo");
}

#[test]
fn test_type_hierarchy_supertypes_self_as_trait_not_matched() {
    let id = SUPER_P9_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_p9_roleinv_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("roleinv_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 1,
        is_macro_expansion: false,
        name: "MyTrait".to_string(),
        path: "roleinv_crate::MyTrait".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 17,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Struct,
        visibility: 1,
        is_macro_expansion: false,
        name: "MyTarget".to_string(),
        path: "roleinv_crate::MyTarget".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 2,
        start_col: 1,
        end_line: 2,
        end_col: 18,
    });
    writer.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Impl,
        visibility: 1,
        is_macro_expansion: false,
        name: "<MyTarget as MyTrait>".to_string(),
        path: "<MyTarget as MyTrait>".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 4,
        start_col: 1,
        end_line: 4,
        end_col: 28,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let code = "trait MyTrait {}\nstruct MyTarget;\n\nimpl MyTrait for MyTarget {}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    // Asking for supertypes of MyTrait (traits don't have supertypes via impl Trait for Struct)
    let req = json!({
        "jsonrpc": "2.0", "id": 822, "method": "typeHierarchy/supertypes",
        "params": { "item": { "name": "MyTrait", "kind": 11, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 822);
    assert_eq!(resp["result"], json!([]));
}
