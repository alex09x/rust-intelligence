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
    writer.add_definition(rdef(1, DefKind::Fn, 1, false, "target", "beyond_crate::target", None, &lib_str, 1, 1, 1, 7));
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
    writer.add_definition(rdef(1, DefKind::Fn, 1, false, "dest_a", "consec_crate::dest_a", None, &lib_str, 1, 1, 1, 7));
    writer.add_definition(rdef(2, DefKind::Fn, 1, false, "dest_b", "consec_crate::dest_b", None, &lib_str, 3, 1, 3, 7));
    writer.add_definition(rdef(3, DefKind::Fn, 1, false, "caller_for_a", "consec_crate::caller_for_a", None, &lib_str, 5, 1, 5, 13));
    writer.add_definition(rdef(4, DefKind::Fn, 1, false, "caller_for_b", "consec_crate::caller_for_b", None, &lib_str, 9, 1, 9, 13));
    writer.add_reference(rref(&lib_str, 6, 5, 6, 11, Some(1), "consec_crate::dest_a", Some(lib_str.clone()), None, 1, 1, DefKind::Fn, false, false));
    writer.add_reference(rref(&lib_str, 10, 5, 10, 11, Some(2), "consec_crate::dest_b", Some(lib_str.clone()), None, 3, 1, DefKind::Fn, false, false));
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
    writer.add_definition(rdef(1, DefKind::Fn, 1, false, "target", "eof_crate::target", None, &lib_str, 1, 1, 1, 7));
    writer.add_definition(rdef(2, DefKind::Fn, 1, false, "eof_caller", "eof_crate::eof_caller", None, &lib_str, 3, 1, 3, 11));
    writer.add_reference(rref(&lib_str, 4, 5, 4, 11, Some(1), "eof_crate::target", Some(lib_str.clone()), None, 1, 1, DefKind::Fn, false, false));
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
    writer.add_definition(rdef(1, DefKind::Fn, 1, false, "fn_c", "chain_crate::fn_c", None, &lib_str, 1, 1, 1, 5));
    writer.add_definition(rdef(2, DefKind::Fn, 1, false, "fn_b", "chain_crate::fn_b", None, &lib_str, 3, 1, 3, 5));
    writer.add_definition(rdef(3, DefKind::Fn, 1, false, "fn_a", "chain_crate::fn_a", None, &lib_str, 7, 1, 7, 5));
    writer.add_reference(rref(&lib_str, 4, 5, 4, 9, Some(1), "chain_crate::fn_c", Some(lib_str.clone()), None, 1, 1, DefKind::Fn, false, false));
    writer.add_reference(rref(&lib_str, 8, 5, 8, 9, Some(2), "chain_crate::fn_b", Some(lib_str.clone()), None, 3, 1, DefKind::Fn, false, false));
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
