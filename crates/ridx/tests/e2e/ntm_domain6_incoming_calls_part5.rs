/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for callHierarchy/incomingCalls (Domain 6, Part 5).
//! Tests multiple caller files, direct recursive calls, mutually recursive callers,
//! and callers located inside child modules.

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

static INC_COUNTER_P5: AtomicU64 = AtomicU64::new(1500);

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
fn test_call_hierarchy_incoming_calls_two_files_callers() {
    let id = INC_COUNTER_P5.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_inc_p5_twofiles_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let lib_file = dir.join("src/lib.rs");
    let a_file = dir.join("src/a.rs");
    let b_file = dir.join("src/b.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();
    let a_str = a_file.to_str().unwrap().to_string();
    let b_str = b_file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("two_files_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "common_fn".to_string(),
        path: "two_files_crate::common_fn".to_string(),
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
        name: "caller_in_a".to_string(),
        path: "two_files_crate::a::caller_in_a".to_string(),
        signature: None,
        file: a_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 12,
    });
    writer.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "caller_in_b".to_string(),
        path: "two_files_crate::b::caller_in_b".to_string(),
        signature: None,
        file: b_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 12,
    });
    writer.add_reference(RawReference {
        file: a_str.clone(),
        start_line: 2,
        start_col: 5,
        end_line: 2,
        end_col: 14,
        target_def_id: Some(1),
        target_path: "two_files_crate::common_fn".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.add_reference(RawReference {
        file: b_str.clone(),
        start_line: 2,
        start_col: 5,
        end_line: 2,
        end_col: 14,
        target_def_id: Some(1),
        target_path: "two_files_crate::common_fn".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&lib_file, "pub fn common_fn() {}\n").unwrap();
    std::fs::write(&a_file, "pub fn caller_in_a() {\n    common_fn();\n}\n").unwrap();
    std::fs::write(&b_file, "pub fn caller_in_b() {\n    common_fn();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 665, "method": "callHierarchy/incomingCalls",
        "params": {
            "item": {
                "name": "common_fn", "kind": 12, "uri": to_uri(&lib_file),
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 9 } },
                "selectionRange": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 9 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 665);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 2);
}

#[test]
fn test_call_hierarchy_incoming_calls_recursive_self_caller() {
    let id = INC_COUNTER_P5.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_inc_p5_rec_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("recursive_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "factorial".to_string(),
        path: "recursive_crate::factorial".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 3,
        start_col: 9,
        end_line: 3,
        end_col: 18,
        target_def_id: Some(1),
        target_path: "recursive_crate::factorial".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn factorial(n: u64) -> u64 {\n    if n <= 1 { 1 } else {\n        factorial(n - 1)\n    }\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 666, "method": "callHierarchy/incomingCalls",
        "params": {
            "item": {
                "name": "factorial", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 9 } },
                "selectionRange": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 9 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 666);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["from"]["name"], "factorial");
}

#[test]
fn test_call_hierarchy_incoming_calls_mutually_recursive_functions() {
    let id = INC_COUNTER_P5.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_inc_p5_mutrec_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("mut_rec_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "ping".to_string(),
        path: "mut_rec_crate::ping".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 5,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "pong".to_string(),
        path: "mut_rec_crate::pong".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 5,
        start_col: 1,
        end_line: 5,
        end_col: 5,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 2,
        start_col: 5,
        end_line: 2,
        end_col: 9,
        target_def_id: Some(2),
        target_path: "mut_rec_crate::pong".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 5,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 6,
        start_col: 5,
        end_line: 6,
        end_col: 9,
        target_def_id: Some(1),
        target_path: "mut_rec_crate::ping".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn ping() {\n    pong();\n}\n\nfn pong() {\n    ping();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 667, "method": "callHierarchy/incomingCalls",
        "params": {
            "item": {
                "name": "ping", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 4 } },
                "selectionRange": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 4 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 667);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["from"]["name"], "pong");
}

#[test]
fn test_call_hierarchy_incoming_calls_caller_in_submodule() {
    let id = INC_COUNTER_P5.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_inc_p5_submod_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("submod_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "top_callee".to_string(),
        path: "submod_crate::top_callee".to_string(),
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
        name: "inner_worker".to_string(),
        path: "submod_crate::nested::inner_worker".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 4,
        start_col: 5,
        end_line: 4,
        end_col: 25,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 5,
        start_col: 9,
        end_line: 5,
        end_col: 19,
        target_def_id: Some(1),
        target_path: "submod_crate::top_callee".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn top_callee() {}\n\nmod nested {\n    pub fn inner_worker() {\n        top_callee();\n    }\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 668, "method": "callHierarchy/incomingCalls",
        "params": {
            "item": {
                "name": "top_callee", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 10 } },
                "selectionRange": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 10 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 668);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["from"]["name"], "inner_worker");
}
