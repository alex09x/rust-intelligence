/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for callHierarchy/outgoingCalls (Domain 6, Part 7).
//! Tests trait method targets, generic function targets, start-line-only range saturation,
//! and repeated calls to the same callee.

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

static OUT_COUNTER_P7: AtomicU64 = AtomicU64::new(2700);

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
        dispatch_lsp_message(
            msg.clone(), &mut self.writer, &mut self.lifecycle,
            &mut self.multi_root, &mut self.overlay_mgr, &self.cancelled, None,
        ).unwrap()
    }

    fn parse_response(&mut self) -> Value {
        let text = String::from_utf8(self.writer.clone()).unwrap();
        self.writer.clear();
        let body = text.split("\r\n\r\n").nth(1).unwrap();
        serde_json::from_str(body).unwrap()
    }
}

#[test]
fn test_call_hierarchy_outgoing_calls_trait_default_method_callee() {
    let id = OUT_COUNTER_P7.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_out_p7_trait_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("trait_tgt_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::AssocFn, visibility: 1, is_macro_expansion: false,
        name: "hook".to_string(), path: "trait_tgt_crate::Handler::hook".to_string(),
        signature: Some("fn hook(&self)".to_string()), file: lib_str.clone(),
        start_line: 2, start_col: 5, end_line: 2, end_col: 9,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "invoke_hook".to_string(), path: "trait_tgt_crate::invoke_hook".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 5, start_col: 1, end_line: 7, end_col: 2,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(), start_line: 6, start_col: 5, end_line: 6, end_col: 9,
        target_def_id: Some(1), target_path: "trait_tgt_crate::Handler::hook".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None, target_line: 2, target_col: 5,
        kind: DefKind::AssocFn, is_write: false, is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "trait Handler {\n    fn hook(&self) {}\n}\n\nfn invoke_hook() {\n    hook();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 731, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "invoke_hook", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 4, "character": 0 }, "end": { "line": 6, "character": 1 } },
                "selectionRange": { "start": { "line": 4, "character": 0 }, "end": { "line": 6, "character": 1 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 731);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["to"]["name"], "hook");
    assert_eq!(calls[0]["to"]["kind"], 6);
}

#[test]
fn test_call_hierarchy_outgoing_calls_generic_fn_callee() {
    let id = OUT_COUNTER_P7.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_out_p7_gen_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("gen_tgt_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "consume".to_string(), path: "gen_tgt_crate::consume".to_string(),
        signature: Some("pub fn consume<T>(val: T)".to_string()), file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 8,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "call_generic".to_string(), path: "gen_tgt_crate::call_generic".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 3, start_col: 1, end_line: 5, end_col: 2,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(), start_line: 4, start_col: 5, end_line: 4, end_col: 12,
        target_def_id: Some(1), target_path: "gen_tgt_crate::consume".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None, target_line: 1, target_col: 1,
        kind: DefKind::Fn, is_write: false, is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn consume<T>(_val: T) {}\n\nfn call_generic() {\n    consume(123);\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 732, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "call_generic", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 2, "character": 0 }, "end": { "line": 4, "character": 1 } },
                "selectionRange": { "start": { "line": 2, "character": 0 }, "end": { "line": 4, "character": 1 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 732);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["to"]["name"], "consume");
}

#[test]
fn test_call_hierarchy_outgoing_calls_caller_range_only_start_line() {
    let id = OUT_COUNTER_P7.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_out_p7_sat_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("sat_range_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "dest".to_string(), path: "sat_range_crate::dest".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 5,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "caller".to_string(), path: "sat_range_crate::caller".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 3, start_col: 1, end_line: 5, end_col: 2,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(), start_line: 4, start_col: 5, end_line: 4, end_col: 9,
        target_def_id: Some(1), target_path: "sat_range_crate::dest".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None, target_line: 1, target_col: 1,
        kind: DefKind::Fn, is_write: false, is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn dest() {}\n\nfn caller() {\n    dest();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 733, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "caller", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 2, "character": 0 }, "end": { "line": 2, "character": 9 } },
                "selectionRange": { "start": { "line": 2, "character": 0 }, "end": { "line": 2, "character": 9 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 733);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["to"]["name"], "dest");
}

#[test]
fn test_call_hierarchy_outgoing_calls_three_calls_to_same_callee() {
    let id = OUT_COUNTER_P7.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_out_p7_three_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("three_calls_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "ping".to_string(), path: "three_calls_crate::ping".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 5,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "pinger".to_string(), path: "three_calls_crate::pinger".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 3, start_col: 1, end_line: 7, end_col: 2,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(), start_line: 4, start_col: 5, end_line: 4, end_col: 9,
        target_def_id: Some(1), target_path: "three_calls_crate::ping".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None, target_line: 1, target_col: 1,
        kind: DefKind::Fn, is_write: false, is_macro_expansion: false,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(), start_line: 5, start_col: 5, end_line: 5, end_col: 9,
        target_def_id: Some(1), target_path: "three_calls_crate::ping".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None, target_line: 1, target_col: 1,
        kind: DefKind::Fn, is_write: false, is_macro_expansion: false,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(), start_line: 6, start_col: 5, end_line: 6, end_col: 9,
        target_def_id: Some(1), target_path: "three_calls_crate::ping".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None, target_line: 1, target_col: 1,
        kind: DefKind::Fn, is_write: false, is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn ping() {}\n\nfn pinger() {\n    ping();\n    ping();\n    ping();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 734, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "pinger", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 2, "character": 0 }, "end": { "line": 6, "character": 1 } },
                "selectionRange": { "start": { "line": 2, "character": 0 }, "end": { "line": 6, "character": 1 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 734);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0]["to"]["name"], "ping");
    assert_eq!(calls[1]["to"]["name"], "ping");
    assert_eq!(calls[2]["to"]["name"], "ping");
}
