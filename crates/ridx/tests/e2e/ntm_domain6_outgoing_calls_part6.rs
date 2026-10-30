/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for callHierarchy/outgoingCalls (Domain 6, Part 6).
//! Tests async and unsafe callee functions, private callees, and pub(crate) callees.

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

static OUT_COUNTER_P6: AtomicU64 = AtomicU64::new(2600);

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
fn test_call_hierarchy_outgoing_calls_async_fn_callee() {
    let id = OUT_COUNTER_P6.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_out_p6_async_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("async_callee_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "background_task".to_string(),
        path: "async_callee_crate::background_task".to_string(),
        signature: Some("async fn background_task()".to_string()),
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 16,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "spawner".to_string(),
        path: "async_callee_crate::spawner".to_string(),
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
        end_col: 20,
        target_def_id: Some(1),
        target_path: "async_callee_crate::background_task".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "async fn background_task() {}\n\nfn spawner() {\n    background_task();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 721, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "spawner", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 2, "character": 0 }, "end": { "line": 4, "character": 1 } },
                "selectionRange": { "start": { "line": 2, "character": 0 }, "end": { "line": 4, "character": 1 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 721);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["to"]["name"], "background_task");
}

#[test]
fn test_call_hierarchy_outgoing_calls_unsafe_fn_callee() {
    let id = OUT_COUNTER_P6.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_out_p6_unsafe_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("unsafe_callee_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "dangerous_op".to_string(),
        path: "unsafe_callee_crate::dangerous_op".to_string(),
        signature: Some("unsafe fn dangerous_op()".to_string()),
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 13,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "wrapper".to_string(),
        path: "unsafe_callee_crate::wrapper".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 3,
        start_col: 1,
        end_line: 7,
        end_col: 2,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 5,
        start_col: 9,
        end_line: 5,
        end_col: 21,
        target_def_id: Some(1),
        target_path: "unsafe_callee_crate::dangerous_op".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "unsafe fn dangerous_op() {}\n\nfn wrapper() {\n    unsafe {\n        dangerous_op();\n    }\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 722, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "wrapper", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 2, "character": 0 }, "end": { "line": 6, "character": 1 } },
                "selectionRange": { "start": { "line": 2, "character": 0 }, "end": { "line": 6, "character": 1 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 722);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["to"]["name"], "dangerous_op");
}

#[test]
fn test_call_hierarchy_outgoing_calls_private_callee() {
    let id = OUT_COUNTER_P6.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_out_p6_priv_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("priv_callee_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "internal_logic".to_string(),
        path: "priv_callee_crate::internal_logic".to_string(),
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
        name: "entrypoint".to_string(),
        path: "priv_callee_crate::entrypoint".to_string(),
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
        end_col: 19,
        target_def_id: Some(1),
        target_path: "priv_callee_crate::internal_logic".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn internal_logic() {}\n\npub fn entrypoint() {\n    internal_logic();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 723, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "entrypoint", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 2, "character": 0 }, "end": { "line": 4, "character": 1 } },
                "selectionRange": { "start": { "line": 2, "character": 0 }, "end": { "line": 4, "character": 1 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 723);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["to"]["name"], "internal_logic");
}

#[test]
fn test_call_hierarchy_outgoing_calls_pub_crate_callee() {
    let id = OUT_COUNTER_P6.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_out_p6_pubcr_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("pubcr_callee_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 2,
        is_macro_expansion: false,
        name: "crate_fn".to_string(),
        path: "pubcr_callee_crate::crate_fn".to_string(),
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
        name: "public_caller".to_string(),
        path: "pubcr_callee_crate::public_caller".to_string(),
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
        target_path: "pubcr_callee_crate::crate_fn".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "pub(crate) fn crate_fn() {}\n\npub fn public_caller() {\n    crate_fn();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 724, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "public_caller", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 2, "character": 0 }, "end": { "line": 4, "character": 1 } },
                "selectionRange": { "start": { "line": 2, "character": 0 }, "end": { "line": 4, "character": 1 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 724);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["to"]["name"], "crate_fn");
}
