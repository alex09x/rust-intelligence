/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for callHierarchy/incomingCalls (Domain 6, Part 1).
//! Tests single and multiple callers, non-call references, overlay call sites, and ID echoing.

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

static INC_COUNTER: AtomicU64 = AtomicU64::new(1);

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
    let id = INC_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_inccall_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let lib_file = dir.join("src/lib.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new(name, "0.1.0");
    writer.add_definition(rdef(1, DefKind::Fn, 1, false, "callee", format!("{}::callee", name), None, &lib_str, 1, 1, 1, 7));
    writer.add_definition(rdef(2, DefKind::Fn, 1, false, "caller", format!("{}::caller", name), None, &lib_str, 3, 1, 3, 7));
    writer.add_reference(rref(&lib_str, 4, 5, 4, 11, Some(1), format!("{}::callee", name), Some(lib_str.clone()), None, 1, 1, DefKind::Fn, false, false));
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
fn test_call_hierarchy_incoming_calls_single_caller_in_same_file() {
    let ts = create_temp_shard("single");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn callee() {}\n\nfn caller() {\n    callee();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 621, "method": "callHierarchy/incomingCalls",
        "params": {
            "item": {
                "name": "callee", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 7 } },
                "selectionRange": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 7 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 621);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["from"]["name"], "caller");
}

#[test]
fn test_call_hierarchy_incoming_calls_multiple_callers_in_same_file() {
    let id = INC_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_inccall_m_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("multi_crate", "0.1.0");
    writer.add_definition(rdef(1, DefKind::Fn, 1, false, "callee", "multi_crate::callee", None, &lib_str, 1, 1, 1, 7));
    writer.add_definition(rdef(2, DefKind::Fn, 1, false, "caller_one", "multi_crate::caller_one", None, &lib_str, 3, 1, 3, 11));
    writer.add_definition(rdef(3, DefKind::Fn, 1, false, "caller_two", "multi_crate::caller_two", None, &lib_str, 7, 1, 7, 11));
    writer.add_reference(rref(&lib_str, 4, 5, 4, 11, Some(1), "multi_crate::callee", Some(lib_str.clone()), None, 1, 1, DefKind::Fn, false, false));
    writer.add_reference(rref(&lib_str, 8, 5, 8, 11, Some(1), "multi_crate::callee", Some(lib_str.clone()), None, 1, 1, DefKind::Fn, false, false));
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn callee() {}\n\nfn caller_one() {\n    callee();\n}\n\nfn caller_two() {\n    callee();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 622, "method": "callHierarchy/incomingCalls",
        "params": {
            "item": {
                "name": "callee", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 7 } },
                "selectionRange": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 7 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 2);
}

#[test]
fn test_call_hierarchy_incoming_calls_multiple_calls_from_same_caller() {
    let id = INC_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_inccall_same_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("same_caller_crate", "0.1.0");
    writer.add_definition(rdef(1, DefKind::Fn, 1, false, "callee", "same_caller_crate::callee", None, &lib_str, 1, 1, 1, 7));
    writer.add_definition(rdef(2, DefKind::Fn, 1, false, "caller", "same_caller_crate::caller", None, &lib_str, 3, 1, 3, 7));
    writer.add_reference(rref(&lib_str, 4, 5, 4, 11, Some(1), "same_caller_crate::callee", Some(lib_str.clone()), None, 1, 1, DefKind::Fn, false, false));
    writer.add_reference(rref(&lib_str, 5, 5, 5, 11, Some(1), "same_caller_crate::callee", Some(lib_str.clone()), None, 1, 1, DefKind::Fn, false, false));
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn callee() {}\n\nfn caller() {\n    callee();\n    callee();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 623, "method": "callHierarchy/incomingCalls",
        "params": {
            "item": {
                "name": "callee", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 7 } },
                "selectionRange": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 7 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    let from_ranges = calls[0]["fromRanges"].as_array().unwrap();
    assert_eq!(from_ranges.len(), 2);
}

#[test]
fn test_call_hierarchy_incoming_calls_zero_callers_returns_empty() {
    let ts = create_temp_shard("zero_calls");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn callee() {}\n\nfn caller() {\n    callee();\n}\nfn isolated() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    // Query on isolated function at line 6 (0-based line 5)
    let req = json!({
        "jsonrpc": "2.0", "id": 624, "method": "callHierarchy/incomingCalls",
        "params": {
            "item": {
                "name": "isolated", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 5, "character": 0 }, "end": { "line": 5, "character": 8 } },
                "selectionRange": { "start": { "line": 5, "character": 0 }, "end": { "line": 5, "character": 8 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 0);
    // Positive control query on callee (has 1 caller)
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 6240, "method": "callHierarchy/incomingCalls",
        "params": {
            "item": {
                "name": "callee", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 7 } },
                "selectionRange": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 7 } }
            }
        }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_calls = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_calls.len(), 1);
}

#[test]
fn test_call_hierarchy_incoming_calls_non_call_reference_ignored() {
    let id = INC_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_inccall_noncall_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("noncall_crate", "0.1.0");
    writer.add_definition(rdef(1, DefKind::Fn, 1, false, "target", "noncall_crate::target", None, &lib_str, 1, 1, 1, 7));
    writer.add_definition(rdef(2, DefKind::Fn, 1, false, "holder", "noncall_crate::holder", None, &lib_str, 3, 1, 3, 7));
    // Reference on line 4 without parenthesis: `let _ptr = target;`
    writer.add_reference(rref(&lib_str, 4, 15, 4, 21, Some(1), "noncall_crate::target", Some(lib_str.clone()), None, 1, 1, DefKind::Fn, false, false));
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn target() {}\n\nfn holder() {\n    let _ptr = target;\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 625, "method": "callHierarchy/incomingCalls",
        "params": {
            "item": {
                "name": "target", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 7 } },
                "selectionRange": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 7 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 0);
    // Positive control by opening overlay with actual call site
    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn target() {}\n\nfn holder() {\n    let _ptr = target();\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));
    assert!(dsp.dispatch(&req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_calls = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_calls.len(), 1);
}
