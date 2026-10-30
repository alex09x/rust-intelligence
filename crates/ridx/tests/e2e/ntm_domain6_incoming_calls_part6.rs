/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for callHierarchy/incomingCalls (Domain 6, Part 6).
//! Tests callers with explicit signatures and function qualifiers (const, async, unsafe).

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

static INC_COUNTER_P6: AtomicU64 = AtomicU64::new(1600);

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
fn test_call_hierarchy_incoming_calls_caller_with_signature() {
    let id = INC_COUNTER_P6.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_inc_p6_sig_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("sig_caller_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "service_target".to_string(),
        path: "sig_caller_crate::service_target".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 15,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "worker".to_string(),
        path: "sig_caller_crate::worker".to_string(),
        signature: Some("pub fn worker(val: u32) -> bool".to_string()),
        file: lib_str.clone(),
        start_line: 3,
        start_col: 1,
        end_line: 3,
        end_col: 7,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 4,
        start_col: 5,
        end_line: 4,
        end_col: 19,
        target_def_id: Some(1),
        target_path: "sig_caller_crate::service_target".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn service_target() {}\n\npub fn worker(val: u32) -> bool {\n    service_target();\n    true\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 671, "method": "callHierarchy/incomingCalls",
        "params": {
            "item": {
                "name": "service_target", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 14 } },
                "selectionRange": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 14 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 671);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["from"]["name"], "worker");
    assert_eq!(calls[0]["from"]["detail"], "pub fn worker(val: u32) -> bool");
}

#[test]
fn test_call_hierarchy_incoming_calls_const_fn_caller() {
    let id = INC_COUNTER_P6.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_inc_p6_const_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("const_caller_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "const_leaf".to_string(),
        path: "const_caller_crate::const_leaf".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 11,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "calc_const".to_string(),
        path: "const_caller_crate::calc_const".to_string(),
        signature: Some("const fn calc_const() -> i32".to_string()),
        file: lib_str.clone(),
        start_line: 3,
        start_col: 1,
        end_line: 3,
        end_col: 11,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 4,
        start_col: 5,
        end_line: 4,
        end_col: 15,
        target_def_id: Some(1),
        target_path: "const_caller_crate::const_leaf".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "const fn const_leaf() -> i32 { 1 }\n\nconst fn calc_const() -> i32 {\n    const_leaf();\n    2\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 672, "method": "callHierarchy/incomingCalls",
        "params": {
            "item": {
                "name": "const_leaf", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 10 } },
                "selectionRange": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 10 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 672);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["from"]["name"], "calc_const");
}

#[test]
fn test_call_hierarchy_incoming_calls_async_fn_caller() {
    let id = INC_COUNTER_P6.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_inc_p6_async_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("async_caller_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "sync_task".to_string(),
        path: "async_caller_crate::sync_task".to_string(),
        signature: None,
        file: lib_str.clone(),
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
        name: "async_runner".to_string(),
        path: "async_caller_crate::async_runner".to_string(),
        signature: Some("async fn async_runner()".to_string()),
        file: lib_str.clone(),
        start_line: 3,
        start_col: 1,
        end_line: 3,
        end_col: 13,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 4,
        start_col: 5,
        end_line: 4,
        end_col: 14,
        target_def_id: Some(1),
        target_path: "async_caller_crate::sync_task".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn sync_task() {}\n\nasync fn async_runner() {\n    sync_task();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 673, "method": "callHierarchy/incomingCalls",
        "params": {
            "item": {
                "name": "sync_task", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 9 } },
                "selectionRange": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 9 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 673);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["from"]["name"], "async_runner");
}

#[test]
fn test_call_hierarchy_incoming_calls_unsafe_fn_caller() {
    let id = INC_COUNTER_P6.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_inc_p6_unsafe_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("unsafe_caller_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "safe_op".to_string(),
        path: "unsafe_caller_crate::safe_op".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 8,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "raw_invoker".to_string(),
        path: "unsafe_caller_crate::raw_invoker".to_string(),
        signature: Some("unsafe fn raw_invoker()".to_string()),
        file: lib_str.clone(),
        start_line: 3,
        start_col: 1,
        end_line: 3,
        end_col: 12,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 4,
        start_col: 5,
        end_line: 4,
        end_col: 12,
        target_def_id: Some(1),
        target_path: "unsafe_caller_crate::safe_op".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn safe_op() {}\n\nunsafe fn raw_invoker() {\n    safe_op();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 674, "method": "callHierarchy/incomingCalls",
        "params": {
            "item": {
                "name": "safe_op", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 7 } },
                "selectionRange": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 7 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 674);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["from"]["name"], "raw_invoker");
}
