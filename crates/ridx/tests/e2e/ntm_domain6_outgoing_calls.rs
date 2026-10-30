/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for callHierarchy/outgoingCalls (Domain 6, Part 1).
//! Tests single callee, multiple callees, repeated calls, zero calls, recursive call filtering, and ID echoing.

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

static OUT_COUNTER: AtomicU64 = AtomicU64::new(1);

struct TempShard {
    dir: PathBuf,
    shard: PathBuf,
}

impl Drop for TempShard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[allow(clippy::too_many_arguments, dead_code)]
fn rdef(
    id: u32,
    kind: DefKind,
    vis: u8,
    mac: bool,
    name: impl Into<String>,
    path: impl Into<String>,
    sig: Option<String>,
    file: impl Into<String>,
    sl: u32,
    sc: u32,
    el: u32,
    ec: u32,
) -> RawDefinition {
    RawDefinition {
        def_id: id,
        kind,
        visibility: vis,
        is_macro_expansion: mac,
        name: name.into(),
        path: path.into(),
        signature: sig,
        file: file.into(),
        start_line: sl,
        start_col: sc,
        end_line: el,
        end_col: ec,
    }
}

#[allow(clippy::too_many_arguments, dead_code)]
fn rref(
    file: impl Into<String>,
    sl: u32,
    sc: u32,
    el: u32,
    ec: u32,
    td: Option<u32>,
    tp: impl Into<String>,
    tf: Option<String>,
    tu: Option<String>,
    tl: u32,
    tc: u32,
    kind: DefKind,
    w: bool,
    m: bool,
) -> RawReference {
    RawReference {
        file: file.into(),
        start_line: sl,
        start_col: sc,
        end_line: el,
        end_col: ec,
        target_def_id: td,
        target_path: tp.into(),
        target_file: tf,
        target_unit: tu,
        target_line: tl,
        target_col: tc,
        kind,
        is_write: w,
        is_macro_expansion: m,
    }
}

fn create_temp_shard(name: &str) -> TempShard {
    let id = OUT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_outcall_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let lib_file = dir.join("src/lib.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new(name, "0.1.0");
    writer.add_definition(rdef(1, DefKind::Fn, 1, false, "callee", format!("{}::callee", name), Some("pub fn callee()".to_string()), &lib_str, 1, 1, 1, 7));
    writer.add_definition(rdef(2, DefKind::Fn, 1, false, "caller", format!("{}::caller", name), Some("pub fn caller()".to_string()), &lib_str, 3, 1, 6, 2));
    writer.add_reference(rref(&lib_str, 4, 5, 4, 11, Some(1), format!("{}::callee", name), Some(lib_str.clone()), None, 1, 1, DefKind::Fn, false, false));
    writer.add_reference(rref(&lib_str, 5, 5, 5, 11, Some(1), format!("{}::callee", name), Some(lib_str.clone()), None, 1, 1, DefKind::Fn, false, false));
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
fn test_call_hierarchy_outgoing_calls_single_callee() {
    let ts = create_temp_shard("single");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn callee() {}\n\nfn caller() {\n    callee();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 651, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "caller", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 2, "character": 0 }, "end": { "line": 4, "character": 1 } },
                "selectionRange": { "start": { "line": 2, "character": 0 }, "end": { "line": 4, "character": 1 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 651);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["to"]["name"], "callee");
    assert_eq!(calls[0]["to"]["kind"], 12);
    let from_ranges = calls[0]["fromRanges"].as_array().unwrap();
    assert_eq!(from_ranges.len(), 1);
}

#[test]
fn test_call_hierarchy_outgoing_calls_multiple_callees() {
    let id = OUT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_outcall_m_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("multi_out_crate", "0.1.0");
    writer.add_definition(rdef(1, DefKind::Fn, 1, false, "foo", "multi_out_crate::foo", None, &lib_str, 1, 1, 1, 4));
    writer.add_definition(rdef(2, DefKind::Fn, 1, false, "bar", "multi_out_crate::bar", None, &lib_str, 2, 1, 2, 4));
    writer.add_definition(rdef(3, DefKind::Fn, 1, false, "caller", "multi_out_crate::caller", None, &lib_str, 4, 1, 7, 2));
    writer.add_reference(rref(&lib_str, 5, 5, 5, 8, Some(1), "multi_out_crate::foo", Some(lib_str.clone()), None, 1, 1, DefKind::Fn, false, false));
    writer.add_reference(rref(&lib_str, 6, 5, 6, 8, Some(2), "multi_out_crate::bar", Some(lib_str.clone()), None, 2, 1, DefKind::Fn, false, false));
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn foo() {}\nfn bar() {}\n\nfn caller() {\n    foo();\n    bar();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 652, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "caller", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 3, "character": 0 }, "end": { "line": 6, "character": 1 } },
                "selectionRange": { "start": { "line": 3, "character": 0 }, "end": { "line": 6, "character": 1 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["to"]["name"], "foo");
    assert_eq!(calls[1]["to"]["name"], "bar");
}

#[test]
fn test_call_hierarchy_outgoing_calls_multiple_calls_same_callee() {
    let ts = create_temp_shard("same_callee");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn callee() {}\n\nfn caller() {\n    callee();\n    callee();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 653, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "caller", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 2, "character": 0 }, "end": { "line": 5, "character": 1 } },
                "selectionRange": { "start": { "line": 2, "character": 0 }, "end": { "line": 5, "character": 1 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["to"]["name"], "callee");
    assert_eq!(calls[1]["to"]["name"], "callee");
}

#[test]
fn test_call_hierarchy_outgoing_calls_zero_callees_returns_empty() {
    let ts = create_temp_shard("zero");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn callee() {}\n\nfn caller() {\n    callee();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    // callee makes 0 outgoing calls
    let req = json!({
        "jsonrpc": "2.0", "id": 654, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "callee", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 14 } },
                "selectionRange": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 14 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 0);
    // Positive control on caller making 1 call
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 6540, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "caller", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 2, "character": 0 }, "end": { "line": 4, "character": 1 } },
                "selectionRange": { "start": { "line": 2, "character": 0 }, "end": { "line": 4, "character": 1 } }
            }
        }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_calls = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_calls.len(), 1);
}

#[test]
fn test_call_hierarchy_outgoing_calls_recursive_call_self_ignored() {
    let id = OUT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_outcall_rec_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("rec_crate", "0.1.0");
    writer.add_definition(rdef(1, DefKind::Fn, 1, false, "recurse", "rec_crate::recurse", None, &lib_str, 1, 1, 1, 28));
    writer.add_definition(rdef(2, DefKind::Fn, 1, false, "caller", "rec_crate::caller", None, &lib_str, 3, 1, 3, 27));
    writer.add_reference(rref(&lib_str, 1, 15, 1, 22, Some(1), "rec_crate::recurse", Some(lib_str.clone()), None, 1, 1, DefKind::Fn, false, false));
    writer.add_reference(rref(&lib_str, 3, 14, 3, 21, Some(1), "rec_crate::recurse", Some(lib_str.clone()), None, 1, 1, DefKind::Fn, false, false));
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn recurse() { recurse(); }\n\nfn caller() { recurse(); }\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 655, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "recurse", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 27 } },
                "selectionRange": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 27 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let calls = resp["result"].as_array().unwrap();
    // Same-line self-call is filtered out by outgoingCalls logic
    assert_eq!(calls.len(), 0);
    // Positive control on caller calling recurse
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 6550, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "caller", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 2, "character": 0 }, "end": { "line": 2, "character": 26 } },
                "selectionRange": { "start": { "line": 2, "character": 0 }, "end": { "line": 2, "character": 26 } }
            }
        }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_calls = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_calls.len(), 1);
    assert_eq!(ctrl_calls[0]["to"]["name"], "recurse");
}

#[test]
fn test_call_hierarchy_outgoing_calls_dispatcher_echoes_numeric_id() {
    let ts = create_temp_shard("num_id");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn callee() {}\n\nfn caller() {\n    callee();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 888111, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "caller", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 2, "character": 0 }, "end": { "line": 4, "character": 1 } },
                "selectionRange": { "start": { "line": 2, "character": 0 }, "end": { "line": 4, "character": 1 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 888111);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["to"]["name"], "callee");
}

#[test]
fn test_call_hierarchy_outgoing_calls_dispatcher_echoes_string_id() {
    let ts = create_temp_shard("str_id");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn callee() {}\n\nfn caller() {\n    callee();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": "outgoing-str-42", "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "caller", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 2, "character": 0 }, "end": { "line": 4, "character": 1 } },
                "selectionRange": { "start": { "line": 2, "character": 0 }, "end": { "line": 4, "character": 1 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], "outgoing-str-42");
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["to"]["name"], "callee");
}

#[test]
fn test_call_hierarchy_outgoing_calls_valid_uri_missing_range_saturates() {
    let ts = create_temp_shard("miss_range");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn callee() {}\n\nfn caller() {\n    callee();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 888222, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": { "name": "caller", "kind": 12, "uri": to_uri(&file) }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 888222);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["to"]["name"], "callee");
}
