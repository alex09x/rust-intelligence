/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for callHierarchy/incomingCalls (Domain 6, Part 4).
//! Tests method callers, method callees, method-to-method calls, and cross-file resolution.

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

static INC_COUNTER_P4: AtomicU64 = AtomicU64::new(1400);

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
fn test_call_hierarchy_incoming_calls_assoc_method_caller() {
    let id = INC_COUNTER_P4.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_inc_p4_mcaller_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("method_caller_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "target_callee".to_string(),
        path: "method_caller_crate::target_callee".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 14,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::AssocFn,
        visibility: 1,
        is_macro_expansion: false,
        name: "execute".to_string(),
        path: "method_caller_crate::Service::execute".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 5,
        start_col: 5,
        end_line: 5,
        end_col: 12,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 6,
        start_col: 9,
        end_line: 6,
        end_col: 22,
        target_def_id: Some(1),
        target_path: "method_caller_crate::target_callee".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn target_callee() {}\n\nstruct Service;\nimpl Service {\n    fn execute(&self) {\n        target_callee();\n    }\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 661, "method": "callHierarchy/incomingCalls",
        "params": {
            "item": {
                "name": "target_callee", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 13 } },
                "selectionRange": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 13 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 661);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["from"]["name"], "execute");
}

#[test]
fn test_call_hierarchy_incoming_calls_assoc_method_callee() {
    let id = INC_COUNTER_P4.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_inc_p4_mcallee_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("assoc_callee_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::AssocFn,
        visibility: 1,
        is_macro_expansion: false,
        name: "run_job".to_string(),
        path: "assoc_callee_crate::Worker::run_job".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 3,
        start_col: 5,
        end_line: 3,
        end_col: 12,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "main_caller".to_string(),
        path: "assoc_callee_crate::main_caller".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 6,
        start_col: 1,
        end_line: 6,
        end_col: 12,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 7,
        start_col: 5,
        end_line: 7,
        end_col: 12,
        target_def_id: Some(1),
        target_path: "assoc_callee_crate::Worker::run_job".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 3,
        target_col: 5,
        kind: DefKind::AssocFn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "struct Worker;\nimpl Worker {\n    fn run_job(&self) {}\n}\n\nfn main_caller() {\n    run_job();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 662, "method": "callHierarchy/incomingCalls",
        "params": {
            "item": {
                "name": "run_job", "kind": 6, "uri": to_uri(&file),
                "range": { "start": { "line": 2, "character": 4 }, "end": { "line": 2, "character": 11 } },
                "selectionRange": { "start": { "line": 2, "character": 4 }, "end": { "line": 2, "character": 11 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 662);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["from"]["name"], "main_caller");
}

#[test]
fn test_call_hierarchy_incoming_calls_method_to_method() {
    let id = INC_COUNTER_P4.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_inc_p4_m2m_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("m2m_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::AssocFn,
        visibility: 1,
        is_macro_expansion: false,
        name: "step_one".to_string(),
        path: "m2m_crate::Pipeline::step_one".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 3,
        start_col: 5,
        end_line: 3,
        end_col: 13,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::AssocFn,
        visibility: 1,
        is_macro_expansion: false,
        name: "step_two".to_string(),
        path: "m2m_crate::Pipeline::step_two".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 4,
        start_col: 5,
        end_line: 4,
        end_col: 13,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 5,
        start_col: 9,
        end_line: 5,
        end_col: 17,
        target_def_id: Some(1),
        target_path: "m2m_crate::Pipeline::step_one".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 3,
        target_col: 5,
        kind: DefKind::AssocFn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "struct Pipeline;\nimpl Pipeline {\n    fn step_one(&self) {}\n    fn step_two(&self) {\n        step_one();\n    }\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 663, "method": "callHierarchy/incomingCalls",
        "params": {
            "item": {
                "name": "step_one", "kind": 6, "uri": to_uri(&file),
                "range": { "start": { "line": 2, "character": 4 }, "end": { "line": 2, "character": 12 } },
                "selectionRange": { "start": { "line": 2, "character": 4 }, "end": { "line": 2, "character": 12 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 663);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["from"]["name"], "step_two");
}

#[test]
fn test_call_hierarchy_incoming_calls_cross_file_caller() {
    let id = INC_COUNTER_P4.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_inc_p4_cross_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let lib_file = dir.join("src/lib.rs");
    let helper_file = dir.join("src/helper.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();
    let helper_str = helper_file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("cross_file_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "shared_core".to_string(),
        path: "cross_file_crate::shared_core".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 12,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "remote_caller".to_string(),
        path: "cross_file_crate::helper::remote_caller".to_string(),
        signature: None,
        file: helper_str.clone(),
        start_line: 3,
        start_col: 1,
        end_line: 3,
        end_col: 14,
    });
    writer.add_reference(RawReference {
        file: helper_str.clone(),
        start_line: 4,
        start_col: 5,
        end_line: 4,
        end_col: 16,
        target_def_id: Some(1),
        target_path: "cross_file_crate::shared_core".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&lib_file, "pub fn shared_core() {}\n").unwrap();
    std::fs::write(&helper_file, "use crate::shared_core;\n\npub fn remote_caller() {\n    shared_core();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 664, "method": "callHierarchy/incomingCalls",
        "params": {
            "item": {
                "name": "shared_core", "kind": 12, "uri": to_uri(&lib_file),
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 11 } },
                "selectionRange": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 11 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 664);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["from"]["name"], "remote_caller");
    assert_eq!(calls[0]["from"]["uri"], to_uri(&helper_file));
}
