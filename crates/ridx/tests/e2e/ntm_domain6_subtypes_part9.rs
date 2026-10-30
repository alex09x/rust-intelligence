/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for typeHierarchy/subtypes (Domain 6, Part 9).
//! Tests non-impl references, consecutive queries, struct role inversion rejection, and lifetime bounds.

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

static SUB_P9_COUNTER: AtomicU64 = AtomicU64::new(900);

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
fn test_type_hierarchy_subtypes_reference_without_matching_impl_line_ignored() {
    let id = SUB_P9_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_sub_p9_nonimplref_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("nonimplref_sub_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 1,
        is_macro_expansion: false,
        name: "ArgTrait".to_string(),
        path: "nonimplref_sub_crate::ArgTrait".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 19,
    });
    // Reference in a function parameter
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 3,
        start_col: 18,
        end_line: 3,
        end_col: 26,
        target_def_id: Some(1),
        target_path: "nonimplref_sub_crate::ArgTrait".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Trait,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let code = "trait ArgTrait {}\n\nfn process(_x: &dyn ArgTrait) {}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 847, "method": "typeHierarchy/subtypes",
        "params": { "item": { "name": "ArgTrait", "kind": 11, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 847);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_type_hierarchy_subtypes_consecutive_requests_different_traits() {
    let id = SUB_P9_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_sub_p9_consec_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("consec_sub_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 1,
        is_macro_expansion: false,
        name: "FirstTrait".to_string(),
        path: "consec_sub_crate::FirstTrait".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 21,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Trait,
        visibility: 1,
        is_macro_expansion: false,
        name: "SecondTrait".to_string(),
        path: "consec_sub_crate::SecondTrait".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 2,
        start_col: 1,
        end_line: 2,
        end_col: 22,
    });
    writer.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Struct,
        visibility: 1,
        is_macro_expansion: false,
        name: "TargetAlpha".to_string(),
        path: "consec_sub_crate::TargetAlpha".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 3,
        start_col: 1,
        end_line: 3,
        end_col: 20,
    });
    writer.add_definition(RawDefinition {
        def_id: 4,
        kind: DefKind::Struct,
        visibility: 1,
        is_macro_expansion: false,
        name: "TargetBeta".to_string(),
        path: "consec_sub_crate::TargetBeta".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 4,
        start_col: 1,
        end_line: 4,
        end_col: 19,
    });
    writer.add_definition(RawDefinition {
        def_id: 5,
        kind: DefKind::Impl,
        visibility: 1,
        is_macro_expansion: false,
        name: "<TargetAlpha as FirstTrait>".to_string(),
        path: "<TargetAlpha as FirstTrait>".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 6,
        start_col: 1,
        end_line: 6,
        end_col: 33,
    });
    writer.add_definition(RawDefinition {
        def_id: 6,
        kind: DefKind::Impl,
        visibility: 1,
        is_macro_expansion: false,
        name: "<TargetBeta as SecondTrait>".to_string(),
        path: "<TargetBeta as SecondTrait>".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 7,
        start_col: 1,
        end_line: 7,
        end_col: 33,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let code = "trait FirstTrait {}\ntrait SecondTrait {}\nstruct TargetAlpha;\nstruct TargetBeta;\n\nimpl FirstTrait for TargetAlpha {}\nimpl SecondTrait for TargetBeta {}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    // Request 1 for FirstTrait
    let req1 = json!({
        "jsonrpc": "2.0", "id": 848, "method": "typeHierarchy/subtypes",
        "params": { "item": { "name": "FirstTrait", "kind": 11, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req1));
    let resp1 = dsp.parse_response();
    assert_eq!(resp1["id"], 848);
    let items1 = resp1["result"].as_array().unwrap();
    assert_eq!(items1.len(), 1);
    assert_eq!(items1[0]["name"], "TargetAlpha");

    // Request 2 for SecondTrait
    let req2 = json!({
        "jsonrpc": "2.0", "id": 849, "method": "typeHierarchy/subtypes",
        "params": { "item": { "name": "SecondTrait", "kind": 11, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req2));
    let resp2 = dsp.parse_response();
    assert_eq!(resp2["id"], 849);
    let items2 = resp2["result"].as_array().unwrap();
    assert_eq!(items2.len(), 1);
    assert_eq!(items2[0]["name"], "TargetBeta");
}

#[test]
fn test_type_hierarchy_subtypes_self_as_struct_not_matched() {
    let id = SUB_P9_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_sub_p9_roleinv_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("roleinv_sub_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 1,
        is_macro_expansion: false,
        name: "AnyTrait".to_string(),
        path: "roleinv_sub_crate::AnyTrait".to_string(),
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
        name: "MyStruct".to_string(),
        path: "roleinv_sub_crate::MyStruct".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 2,
        start_col: 1,
        end_line: 2,
        end_col: 17,
    });
    writer.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Impl,
        visibility: 1,
        is_macro_expansion: false,
        name: "<MyStruct as AnyTrait>".to_string(),
        path: "<MyStruct as AnyTrait>".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 4,
        start_col: 1,
        end_line: 4,
        end_col: 29,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let code = "trait AnyTrait {}\nstruct MyStruct;\n\nimpl AnyTrait for MyStruct {}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    // Asking for subtypes of a struct name - structs don't have subtypes via `impl Trait for MyStruct`
    let req = json!({
        "jsonrpc": "2.0", "id": 850, "method": "typeHierarchy/subtypes",
        "params": { "item": { "name": "MyStruct", "kind": 23, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 850);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_type_hierarchy_subtypes_trait_with_lifetime_bounds() {
    let id = SUB_P9_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_sub_p9_lt_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("lt_sub_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 1,
        is_macro_expansion: false,
        name: "BorrowedView".to_string(),
        path: "lt_sub_crate::BorrowedView".to_string(),
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
        name: "ViewHolder".to_string(),
        path: "lt_sub_crate::ViewHolder".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 2,
        start_col: 1,
        end_line: 2,
        end_col: 25,
    });
    writer.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Impl,
        visibility: 1,
        is_macro_expansion: false,
        name: "<ViewHolder as BorrowedView>".to_string(),
        path: "<ViewHolder as BorrowedView>".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 4,
        start_col: 1,
        end_line: 4,
        end_col: 51,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();

    let code = "trait BorrowedView<'a> {}\nstruct ViewHolder<'a>(&'a str);\n\nimpl<'a> BorrowedView<'a> for ViewHolder<'a> {}\n";
    std::fs::write(&file, code).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 851, "method": "typeHierarchy/subtypes",
        "params": { "item": { "name": "BorrowedView", "kind": 11, "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 851);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "ViewHolder");
}
