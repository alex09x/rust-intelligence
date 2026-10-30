/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for callHierarchy/outgoingCalls (Domain 6, Part 4).
//! Tests associated method targets, method callers calling free functions,
//! method-to-method outgoing calls, and cross-file callee resolution.

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

static OUT_COUNTER_P4: AtomicU64 = AtomicU64::new(2400);

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
fn test_call_hierarchy_outgoing_calls_assoc_method_target() {
    let id = OUT_COUNTER_P4.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_out_p4_assoctgt_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("assoc_tgt_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::AssocFn,
        visibility: 1,
        is_macro_expansion: false,
        name: "do_work".to_string(),
        path: "assoc_tgt_crate::Worker::do_work".to_string(),
        signature: Some("pub fn do_work(&self)".to_string()),
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
        name: "dispatch_job".to_string(),
        path: "assoc_tgt_crate::dispatch_job".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 6,
        start_col: 1,
        end_line: 8,
        end_col: 2,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 7,
        start_col: 5,
        end_line: 7,
        end_col: 12,
        target_def_id: Some(1),
        target_path: "assoc_tgt_crate::Worker::do_work".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 3,
        target_col: 5,
        kind: DefKind::AssocFn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "struct Worker;\nimpl Worker {\n    fn do_work(&self) {}\n}\n\nfn dispatch_job() {\n    do_work();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 711, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "dispatch_job", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 5, "character": 0 }, "end": { "line": 7, "character": 1 } },
                "selectionRange": { "start": { "line": 5, "character": 0 }, "end": { "line": 7, "character": 1 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 711);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["to"]["name"], "do_work");
    assert_eq!(calls[0]["to"]["kind"], 6);
}

#[test]
fn test_call_hierarchy_outgoing_calls_method_caller_calling_free_fn() {
    let id = OUT_COUNTER_P4.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_out_p4_mcaller_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("mcaller_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "free_helper".to_string(),
        path: "mcaller_crate::free_helper".to_string(),
        signature: Some("pub fn free_helper()".to_string()),
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 12,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::AssocFn,
        visibility: 1,
        is_macro_expansion: false,
        name: "execute".to_string(),
        path: "mcaller_crate::Service::execute".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 5,
        start_col: 5,
        end_line: 7,
        end_col: 6,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 6,
        start_col: 9,
        end_line: 6,
        end_col: 20,
        target_def_id: Some(1),
        target_path: "mcaller_crate::free_helper".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn free_helper() {}\n\nstruct Service;\nimpl Service {\n    fn execute(&self) {\n        free_helper();\n    }\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 712, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "execute", "kind": 6, "uri": to_uri(&file),
                "range": { "start": { "line": 4, "character": 4 }, "end": { "line": 6, "character": 5 } },
                "selectionRange": { "start": { "line": 4, "character": 4 }, "end": { "line": 6, "character": 5 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 712);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["to"]["name"], "free_helper");
    assert_eq!(calls[0]["to"]["kind"], 12);
}

#[test]
fn test_call_hierarchy_outgoing_calls_method_calling_another_method() {
    let id = OUT_COUNTER_P4.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_out_p4_m2m_{}_{}", std::process::id(), id));
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
        end_line: 6,
        end_col: 6,
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
        "jsonrpc": "2.0", "id": 713, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "step_two", "kind": 6, "uri": to_uri(&file),
                "range": { "start": { "line": 3, "character": 4 }, "end": { "line": 5, "character": 5 } },
                "selectionRange": { "start": { "line": 3, "character": 4 }, "end": { "line": 5, "character": 5 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 713);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["to"]["name"], "step_one");
    assert_eq!(calls[0]["to"]["kind"], 6);
}

#[test]
fn test_call_hierarchy_outgoing_calls_cross_file_callee() {
    let id = OUT_COUNTER_P4.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_out_p4_cross_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let lib_file = dir.join("src/lib.rs");
    let other_file = dir.join("src/other.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();
    let other_str = other_file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("cross_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "remote_fn".to_string(),
        path: "cross_crate::other::remote_fn".to_string(),
        signature: None,
        file: other_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "local_caller".to_string(),
        path: "cross_crate::local_caller".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 3,
        end_col: 2,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 2,
        start_col: 5,
        end_line: 2,
        end_col: 14,
        target_def_id: Some(1),
        target_path: "cross_crate::other::remote_fn".to_string(),
        target_file: Some(other_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&lib_file, "fn local_caller() {\n    remote_fn();\n}\n").unwrap();
    std::fs::write(&other_file, "pub fn remote_fn() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 714, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "local_caller", "kind": 12, "uri": to_uri(&lib_file),
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 2, "character": 1 } },
                "selectionRange": { "start": { "line": 0, "character": 0 }, "end": { "line": 2, "character": 1 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 714);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["to"]["name"], "remote_fn");
    assert_eq!(calls[0]["to"]["uri"], to_uri(&other_file));
}
