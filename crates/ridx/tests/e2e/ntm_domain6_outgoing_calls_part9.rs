/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for callHierarchy/outgoingCalls (Domain 6, Part 9).
//! Tests callers at EOF, consecutive queries, out-of-bounds ranges, and unindexed local calls.

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

static OUT_COUNTER_P9: AtomicU64 = AtomicU64::new(2900);

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
fn test_call_hierarchy_outgoing_calls_caller_at_eof() {
    let id = OUT_COUNTER_P9.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_out_p9_eof_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("eof_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "target".to_string(),
        path: "eof_crate::target".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 7,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "eof_fn".to_string(),
        path: "eof_crate::eof_fn".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 3,
        start_col: 1,
        end_line: 3,
        end_col: 22,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 3,
        start_col: 14,
        end_line: 3,
        end_col: 20,
        target_def_id: Some(1),
        target_path: "eof_crate::target".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn target() {}\n\nfn eof_fn() { target(); }").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 751, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "eof_fn", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 2, "character": 0 }, "end": { "line": 2, "character": 24 } },
                "selectionRange": { "start": { "line": 2, "character": 0 }, "end": { "line": 2, "character": 24 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 751);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["to"]["name"], "target");
}

#[test]
fn test_call_hierarchy_outgoing_calls_consecutive_requests_different_callers() {
    let id = OUT_COUNTER_P9.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_out_p9_consec_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("consec_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "target_a".to_string(),
        path: "consec_crate::target_a".to_string(),
        signature: None,
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
        name: "target_b".to_string(),
        path: "consec_crate::target_b".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 3,
        start_col: 1,
        end_line: 3,
        end_col: 9,
    });
    writer.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "caller_a".to_string(),
        path: "consec_crate::caller_a".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 5,
        start_col: 1,
        end_line: 7,
        end_col: 2,
    });
    writer.add_definition(RawDefinition {
        def_id: 4,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "caller_b".to_string(),
        path: "consec_crate::caller_b".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 9,
        start_col: 1,
        end_line: 11,
        end_col: 2,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 6,
        start_col: 5,
        end_line: 6,
        end_col: 13,
        target_def_id: Some(1),
        target_path: "consec_crate::target_a".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 10,
        start_col: 5,
        end_line: 10,
        end_col: 13,
        target_def_id: Some(2),
        target_path: "consec_crate::target_b".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 3,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn target_a() {}\n\nfn target_b() {}\n\nfn caller_a() {\n    target_a();\n}\n\nfn caller_b() {\n    target_b();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req_a = json!({
        "jsonrpc": "2.0", "id": 752, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "caller_a", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 4, "character": 0 }, "end": { "line": 6, "character": 1 } },
                "selectionRange": { "start": { "line": 4, "character": 0 }, "end": { "line": 6, "character": 1 } }
            }
        }
    });
    assert!(dsp.dispatch(&req_a));
    let resp_a = dsp.parse_response();
    assert_eq!(resp_a["id"], 752);
    let calls_a = resp_a["result"].as_array().unwrap();
    assert_eq!(calls_a.len(), 1);
    assert_eq!(calls_a[0]["to"]["name"], "target_a");

    let req_b = json!({
        "jsonrpc": "2.0", "id": 753, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "caller_b", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 8, "character": 0 }, "end": { "line": 10, "character": 1 } },
                "selectionRange": { "start": { "line": 8, "character": 0 }, "end": { "line": 10, "character": 1 } }
            }
        }
    });
    assert!(dsp.dispatch(&req_b));
    let resp_b = dsp.parse_response();
    assert_eq!(resp_b["id"], 753);
    let calls_b = resp_b["result"].as_array().unwrap();
    assert_eq!(calls_b.len(), 1);
    assert_eq!(calls_b[0]["to"]["name"], "target_b");
}

#[test]
fn test_call_hierarchy_outgoing_calls_range_outside_file_returns_empty() {
    let id = OUT_COUNTER_P9.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_out_p9_oob_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("oob_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "simple".to_string(),
        path: "oob_crate::simple".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 7,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn simple() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 754, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "ghost_caller", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 999, "character": 0 }, "end": { "line": 1005, "character": 1 } },
                "selectionRange": { "start": { "line": 999, "character": 0 }, "end": { "line": 1005, "character": 1 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 754);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_call_hierarchy_outgoing_calls_closure_call_ignored_if_not_indexed() {
    let id = OUT_COUNTER_P9.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_out_p9_clos_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("clos_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "outer".to_string(),
        path: "clos_crate::outer".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 9,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn outer() {\n    let f = || 42;\n    f();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 755, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "outer", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 3, "character": 1 } },
                "selectionRange": { "start": { "line": 0, "character": 0 }, "end": { "line": 3, "character": 1 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 755);
    assert_eq!(resp["result"], json!([]));
}
