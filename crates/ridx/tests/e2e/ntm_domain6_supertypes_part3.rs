/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for typeHierarchy/supertypes (Domain 6, Part 3).
//! Tests string IDs, unknown types, empty names, missing workspace, and multiline headers.

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

static SUPER_P3_COUNTER: AtomicU64 = AtomicU64::new(200);

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
    let id = SUPER_P3_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_p3_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let lib_file = dir.join("src/lib.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new(name, "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 1,
        is_macro_expansion: false,
        name: "Greeter".to_string(),
        path: format!("{}::Greeter", name),
        signature: Some("pub trait Greeter".to_string()),
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 18,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Struct,
        visibility: 1,
        is_macro_expansion: false,
        name: "Robot".to_string(),
        path: format!("{}::Robot", name),
        signature: Some("pub struct Robot".to_string()),
        file: lib_str.clone(),
        start_line: 3,
        start_col: 1,
        end_line: 3,
        end_col: 17,
    });
    writer.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Impl,
        visibility: 1,
        is_macro_expansion: false,
        name: "<Robot as Greeter>".to_string(),
        path: "<Robot as Greeter>".to_string(),
        signature: Some("impl Greeter for Robot".to_string()),
        file: lib_str,
        start_line: 5,
        start_col: 1,
        end_line: 7,
        end_col: 2,
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

    fn new_empty() -> Self {
        let mut d = Self {
            lifecycle: LifecycleManager::new(),
            multi_root: MultiRootWorkspace::new(),
            overlay_mgr: OverlayManager::new(),
            cancelled: Arc::new(Mutex::new(HashSet::new())),
            writer: Vec::new(),
        };
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
fn test_type_hierarchy_supertypes_dispatcher_echoes_string_id() {
    let ts = create_temp_shard("str_id");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "trait Greeter {}\n\nstruct Robot;\n\nimpl Greeter for Robot {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": "super-str-88", "method": "typeHierarchy/supertypes",
        "params": {
            "item": { "name": "Robot", "kind": 23, "uri": to_uri(&file) }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], "super-str-88");
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Greeter");
}

#[test]
fn test_type_hierarchy_supertypes_unknown_type_returns_empty() {
    let ts = create_temp_shard("unknown_type");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "trait Greeter {}\n\nstruct Robot;\n\nimpl Greeter for Robot {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 710, "method": "typeHierarchy/supertypes",
        "params": {
            "item": { "name": "UnknownType", "kind": 23, "uri": to_uri(&file) }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 0);

    // Positive control on Robot
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 7100, "method": "typeHierarchy/supertypes",
        "params": {
            "item": { "name": "Robot", "kind": 23, "uri": to_uri(&file) }
        }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_items = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_items.len(), 1);
    assert_eq!(ctrl_items[0]["name"], "Greeter");
}

#[test]
fn test_type_hierarchy_supertypes_empty_name_returns_empty() {
    let ts = create_temp_shard("empty_name");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "trait Greeter {}\n\nstruct Robot;\n\nimpl Greeter for Robot {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 711, "method": "typeHierarchy/supertypes",
        "params": {
            "item": { "name": "", "kind": 23, "uri": to_uri(&file) }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 0);

    // Positive control on Robot
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 7110, "method": "typeHierarchy/supertypes",
        "params": {
            "item": { "name": "Robot", "kind": 23, "uri": to_uri(&file) }
        }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_items = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_items.len(), 1);
    assert_eq!(ctrl_items[0]["name"], "Greeter");
}

#[test]
fn test_type_hierarchy_supertypes_missing_workspace_returns_empty() {
    let mut dsp = TestDispatcher::new_empty();

    let req = json!({
        "jsonrpc": "2.0", "id": 712, "method": "typeHierarchy/supertypes",
        "params": {
            "item": { "name": "Robot", "kind": 23 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 0);

    // Positive control with loaded workspace
    let ts = create_temp_shard("with_ws");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "trait Greeter {}\n\nstruct Robot;\n\nimpl Greeter for Robot {}\n").unwrap();
    let mut ctrl_dsp = TestDispatcher::new(ts.shard.clone());
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 7120, "method": "typeHierarchy/supertypes",
        "params": {
            "item": { "name": "Robot", "kind": 23, "uri": to_uri(&file) }
        }
    });
    assert!(ctrl_dsp.dispatch(&ctrl_req));
    let ctrl_resp = ctrl_dsp.parse_response();
    let ctrl_items = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_items.len(), 1);
    assert_eq!(ctrl_items[0]["name"], "Greeter");
}

#[test]
fn test_type_hierarchy_supertypes_multiline_impl_header() {
    let id = SUPER_P3_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_super_ml_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("ml_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 1,
        is_macro_expansion: false,
        name: "Greeter".to_string(),
        path: "ml_crate::Greeter".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 18,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Struct,
        visibility: 1,
        is_macro_expansion: false,
        name: "Robot".to_string(),
        path: "ml_crate::Robot".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 3,
        start_col: 1,
        end_line: 3,
        end_col: 17,
    });
    writer.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Impl,
        visibility: 1,
        is_macro_expansion: false,
        name: "<Robot as Greeter>".to_string(),
        path: "<Robot as Greeter>".to_string(),
        signature: None,
        file: lib_str,
        start_line: 5,
        start_col: 1,
        end_line: 8,
        end_col: 2,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "trait Greeter {}\n\nstruct Robot;\n\nimpl Greeter\n    for Robot\n{\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 713, "method": "typeHierarchy/supertypes",
        "params": {
            "item": { "name": "Robot", "kind": 23, "uri": to_uri(&file) }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Greeter");
}
