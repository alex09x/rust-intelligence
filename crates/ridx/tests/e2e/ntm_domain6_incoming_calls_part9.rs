/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for callHierarchy/incomingCalls (Domain 6, Part 9).
//! Tests boundary positions, consecutive queries, callers at EOF, and direct caller isolation.

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

static INC_COUNTER_P9: AtomicU64 = AtomicU64::new(1900);


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
fn test_call_hierarchy_incoming_calls_zero_line_zero_col_callee() {
    let id = INC_COUNTER_P9.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_inc_p9_zeropos_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let mut writer = IndexWriter::new("empty_crate", "0.1.0");
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "// empty file\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 686, "method": "callHierarchy/incomingCalls",
        "params": {
            "item": {
                "name": "empty_target", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
                "selectionRange": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 686);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_call_hierarchy_incoming_calls_callee_line_beyond_file_end() {
    let id = INC_COUNTER_P9.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_inc_p9_beyond_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("beyond_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "target".to_string(), path: "beyond_crate::target".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 7,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn target() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 687, "method": "callHierarchy/incomingCalls",
        "params": {
            "item": {
                "name": "target", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 999, "character": 0 }, "end": { "line": 999, "character": 6 } },
                "selectionRange": { "start": { "line": 999, "character": 0 }, "end": { "line": 999, "character": 6 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 687);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_call_hierarchy_incoming_calls_consecutive_requests_different_callees() {
    let id = INC_COUNTER_P9.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_inc_p9_consec_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("consec_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "dest_a".to_string(), path: "consec_crate::dest_a".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 7,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "dest_b".to_string(), path: "consec_crate::dest_b".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 3, start_col: 1, end_line: 3, end_col: 7,
    });
    writer.add_definition(RawDefinition {
        def_id: 3, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "caller_for_a".to_string(), path: "consec_crate::caller_for_a".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 5, start_col: 1, end_line: 5, end_col: 13,
    });
    writer.add_definition(RawDefinition {
        def_id: 4, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "caller_for_b".to_string(), path: "consec_crate::caller_for_b".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 9, start_col: 1, end_line: 9, end_col: 13,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 6, start_col: 5, end_line: 6, end_col: 11,
        target_def_id: Some(1), target_path: "consec_crate::dest_a".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None, target_line: 1, target_col: 1,
        kind: DefKind::Fn, is_write: false, is_macro_expansion: false,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 10, start_col: 5, end_line: 10, end_col: 11,
        target_def_id: Some(2), target_path: "consec_crate::dest_b".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None, target_line: 3, target_col: 1,
        kind: DefKind::Fn, is_write: false, is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn dest_a() {}\n\nfn dest_b() {}\n\nfn caller_for_a() {\n    dest_a();\n}\n\nfn caller_for_b() {\n    dest_b();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req_a = json!({
        "jsonrpc": "2.0", "id": 691, "method": "callHierarchy/incomingCalls",
        "params": {
            "item": {
                "name": "dest_a", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 6 } },
                "selectionRange": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 6 } }
            }
        }
    });
    assert!(dsp.dispatch(&req_a));
    let resp_a = dsp.parse_response();
    assert_eq!(resp_a["id"], 691);
    let calls_a = resp_a["result"].as_array().unwrap();
    assert_eq!(calls_a.len(), 1);
    assert_eq!(calls_a[0]["from"]["name"], "caller_for_a");

    let req_b = json!({
        "jsonrpc": "2.0", "id": 692, "method": "callHierarchy/incomingCalls",
        "params": {
            "item": {
                "name": "dest_b", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 2, "character": 0 }, "end": { "line": 2, "character": 6 } },
                "selectionRange": { "start": { "line": 2, "character": 0 }, "end": { "line": 2, "character": 6 } }
            }
        }
    });
    assert!(dsp.dispatch(&req_b));
    let resp_b = dsp.parse_response();
    assert_eq!(resp_b["id"], 692);
    let calls_b = resp_b["result"].as_array().unwrap();
    assert_eq!(calls_b.len(), 1);
    assert_eq!(calls_b[0]["from"]["name"], "caller_for_b");
}

#[test]
fn test_call_hierarchy_incoming_calls_caller_at_eof() {
    let id = INC_COUNTER_P9.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_inc_p9_eof_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("eof_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "target".to_string(), path: "eof_crate::target".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 7,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "eof_caller".to_string(), path: "eof_crate::eof_caller".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 3, start_col: 1, end_line: 3, end_col: 11,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 4, start_col: 5, end_line: 4, end_col: 11,
        target_def_id: Some(1), target_path: "eof_crate::target".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None, target_line: 1, target_col: 1,
        kind: DefKind::Fn, is_write: false, is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn target() {}\n\nfn eof_caller() {\n    target(); }").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 693, "method": "callHierarchy/incomingCalls",
        "params": {
            "item": {
                "name": "target", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 6 } },
                "selectionRange": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 6 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 693);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["from"]["name"], "eof_caller");
}

#[test]
fn test_call_hierarchy_incoming_calls_direct_caller_only_no_transitive() {
    let id = INC_COUNTER_P9.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_inc_p9_trans_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("chain_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "fn_c".to_string(), path: "chain_crate::fn_c".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 5,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "fn_b".to_string(), path: "chain_crate::fn_b".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 3, start_col: 1, end_line: 3, end_col: 5,
    });
    writer.add_definition(RawDefinition {
        def_id: 3, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "fn_a".to_string(), path: "chain_crate::fn_a".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 7, start_col: 1, end_line: 7, end_col: 5,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 4, start_col: 5, end_line: 4, end_col: 9,
        target_def_id: Some(1), target_path: "chain_crate::fn_c".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None, target_line: 1, target_col: 1,
        kind: DefKind::Fn, is_write: false, is_macro_expansion: false,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 8, start_col: 5, end_line: 8, end_col: 9,
        target_def_id: Some(2), target_path: "chain_crate::fn_b".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None, target_line: 3, target_col: 1,
        kind: DefKind::Fn, is_write: false, is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn fn_c() {}\n\nfn fn_b() {\n    fn_c();\n}\n\nfn fn_a() {\n    fn_b();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 694, "method": "callHierarchy/incomingCalls",
        "params": {
            "item": {
                "name": "fn_c", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 4 } },
                "selectionRange": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 4 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 694);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["from"]["name"], "fn_b");
}
