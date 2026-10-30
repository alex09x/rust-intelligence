/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for callHierarchy/outgoingCalls (Domain 6, Part 5).
//! Tests cross-file callee aggregation, nested control-flow call sites,
//! callee signature propagation, and const fn callee targets.

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

static OUT_COUNTER_P5: AtomicU64 = AtomicU64::new(2500);

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
fn test_call_hierarchy_outgoing_calls_two_callees_in_different_files() {
    let id = OUT_COUNTER_P5.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_out_p5_twocal__{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let lib_file = dir.join("src/lib.rs");
    let a_file = dir.join("src/a.rs");
    let b_file = dir.join("src/b.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();
    let a_str = a_file.to_str().unwrap().to_string();
    let b_str = b_file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("two_callees_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "service_a".to_string(),
        path: "two_callees_crate::a::service_a".to_string(),
        signature: None,
        file: a_str.clone(),
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
        name: "service_b".to_string(),
        path: "two_callees_crate::b::service_b".to_string(),
        signature: None,
        file: b_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });
    writer.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "coordinator".to_string(),
        path: "two_callees_crate::coordinator".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 4,
        end_col: 2,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 2,
        start_col: 5,
        end_line: 2,
        end_col: 14,
        target_def_id: Some(1),
        target_path: "two_callees_crate::a::service_a".to_string(),
        target_file: Some(a_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 3,
        start_col: 5,
        end_line: 3,
        end_col: 14,
        target_def_id: Some(2),
        target_path: "two_callees_crate::b::service_b".to_string(),
        target_file: Some(b_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&lib_file, "fn coordinator() {\n    service_a();\n    service_b();\n}\n").unwrap();
    std::fs::write(&a_file, "pub fn service_a() {}\n").unwrap();
    std::fs::write(&b_file, "pub fn service_b() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 715, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "coordinator", "kind": 12, "uri": to_uri(&lib_file),
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 3, "character": 1 } },
                "selectionRange": { "start": { "line": 0, "character": 0 }, "end": { "line": 3, "character": 1 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 715);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["to"]["name"], "service_a");
    assert_eq!(calls[1]["to"]["name"], "service_b");
}

#[test]
fn test_call_hierarchy_outgoing_calls_deeply_nested_call_site() {
    let id = OUT_COUNTER_P5.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_out_p5_nested_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("nested_site_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "flush".to_string(),
        path: "nested_site_crate::flush".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 6,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "complex_caller".to_string(),
        path: "nested_site_crate::complex_caller".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 3,
        start_col: 1,
        end_line: 9,
        end_col: 2,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 6,
        start_col: 13,
        end_line: 6,
        end_col: 18,
        target_def_id: Some(1),
        target_path: "nested_site_crate::flush".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn flush() {}\n\nfn complex_caller(flag: bool) {\n    if flag {\n        for _ in 0..1 {\n            flush();\n        }\n    }\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 716, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "complex_caller", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 2, "character": 0 }, "end": { "line": 8, "character": 1 } },
                "selectionRange": { "start": { "line": 2, "character": 0 }, "end": { "line": 8, "character": 1 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 716);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["to"]["name"], "flush");
}

#[test]
fn test_call_hierarchy_outgoing_calls_callee_with_signature() {
    let id = OUT_COUNTER_P5.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_out_p5_sig_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("callee_sig_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "math_helper".to_string(),
        path: "callee_sig_crate::math_helper".to_string(),
        signature: Some("pub fn math_helper(x: i32) -> i32".to_string()),
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
        name: "compute".to_string(),
        path: "callee_sig_crate::compute".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 3,
        start_col: 1,
        end_line: 5,
        end_col: 2,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 4,
        start_col: 5,
        end_line: 4,
        end_col: 16,
        target_def_id: Some(1),
        target_path: "callee_sig_crate::math_helper".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn math_helper(x: i32) -> i32 { x }\n\nfn compute() {\n    math_helper(42);\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 717, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "compute", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 2, "character": 0 }, "end": { "line": 4, "character": 1 } },
                "selectionRange": { "start": { "line": 2, "character": 0 }, "end": { "line": 4, "character": 1 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 717);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["to"]["name"], "math_helper");
    assert_eq!(calls[0]["to"]["detail"], "pub fn math_helper(x: i32) -> i32");
}

#[test]
fn test_call_hierarchy_outgoing_calls_const_fn_callee() {
    let id = OUT_COUNTER_P5.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_out_p5_const_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("const_callee_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "const_fn".to_string(),
        path: "const_callee_crate::const_fn".to_string(),
        signature: Some("const fn const_fn() -> u32".to_string()),
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 9,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "runtime_caller".to_string(),
        path: "const_callee_crate::runtime_caller".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 3,
        start_col: 1,
        end_line: 5,
        end_col: 2,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 4,
        start_col: 5,
        end_line: 4,
        end_col: 13,
        target_def_id: Some(1),
        target_path: "const_callee_crate::const_fn".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "const fn const_fn() -> u32 { 10 }\n\nfn runtime_caller() {\n    const_fn();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 718, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "runtime_caller", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 2, "character": 0 }, "end": { "line": 4, "character": 1 } },
                "selectionRange": { "start": { "line": 2, "character": 0 }, "end": { "line": 4, "character": 1 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 718);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["to"]["name"], "const_fn");
}
