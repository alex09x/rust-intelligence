/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for callHierarchy/outgoingCalls (Domain 6, Part 2).
//! Tests turbofish syntax, multiline calls, keyword/declaration filtering, and overlay updates.

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

static OUT_P2_COUNTER: AtomicU64 = AtomicU64::new(100);

struct TempShard {
    dir: PathBuf,
    shard: PathBuf,
}

impl Drop for TempShard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn create_temp_shard(name: &str) -> TempShard {
    let id = OUT_P2_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_outcall_p2_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let lib_file = dir.join("src/lib.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new(name, "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "callee".to_string(), path: format!("{}::callee", name),
        signature: Some("pub fn callee()".to_string()), file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 7,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "caller".to_string(), path: format!("{}::caller", name),
        signature: Some("pub fn caller()".to_string()), file: lib_str.clone(),
        start_line: 3, start_col: 1, end_line: 5, end_col: 2,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 4, start_col: 5, end_line: 4, end_col: 11,
        target_def_id: Some(1), target_path: format!("{}::callee", name),
        target_file: Some(lib_str),
        target_unit: None, target_line: 1, target_col: 1,
        kind: DefKind::Fn, is_write: false, is_macro_expansion: false,
    });
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
fn test_call_hierarchy_outgoing_calls_generic_turbofish_syntax() {
    let id = OUT_P2_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_outcall_turbo_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("turbo_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "parse".to_string(), path: "turbo_crate::parse".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 6,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "caller".to_string(), path: "turbo_crate::caller".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 3, start_col: 1, end_line: 5, end_col: 2,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 4, start_col: 5, end_line: 4, end_col: 10,
        target_def_id: Some(1), target_path: "turbo_crate::parse".to_string(),
        target_file: Some(lib_str),
        target_unit: None, target_line: 1, target_col: 1,
        kind: DefKind::Fn, is_write: false, is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn parse<T>() {}\n\nfn caller() {\n    parse::<i32>();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 658, "method": "callHierarchy/outgoingCalls",
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
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["to"]["name"], "parse");
}

#[test]
fn test_call_hierarchy_outgoing_calls_multiline_call_site() {
    let ts = create_temp_shard("multiline");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn callee() {}\n\nfn caller() {\n    callee(\n    );\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 659, "method": "callHierarchy/outgoingCalls",
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
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["to"]["name"], "callee");
}

#[test]
fn test_call_hierarchy_outgoing_calls_declaration_fn_filtered() {
    let ts = create_temp_shard("decl_filter");
    let file = ts.dir.join("src/lib.rs");
    // Outer caller calls callee() on line 4, and has nested declaration `fn inner() {}` on line 5 which shouldn't be counted as a call to inner
    std::fs::write(&file, "fn callee() {}\n\nfn caller() {\n    callee();\n    fn inner() {}\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 660, "method": "callHierarchy/outgoingCalls",
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
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["to"]["name"], "callee");
}

#[test]
fn test_call_hierarchy_outgoing_calls_keywords_filtered() {
    let ts = create_temp_shard("kw_filter");
    let file = ts.dir.join("src/lib.rs");
    // Outer caller calls callee() on line 4, and has if statement on line 5 which shouldn't be treated as a call
    std::fs::write(&file, "fn callee() {}\n\nfn caller() {\n    callee();\n    if (true) {}\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 661, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "caller", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 2, "character": 0 }, "end": { "line": 6, "character": 1 } },
                "selectionRange": { "start": { "line": 2, "character": 0 }, "end": { "line": 6, "character": 1 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["to"]["name"], "callee");
}

#[test]
fn test_call_hierarchy_outgoing_calls_unknown_callee_ignored() {
    let ts = create_temp_shard("unknown_callee");
    let file = ts.dir.join("src/lib.rs");
    // `unindexed_external()` is called but not in index
    std::fs::write(&file, "fn callee() {}\n\nfn caller() {\n    unindexed_external();\n    callee();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 662, "method": "callHierarchy/outgoingCalls",
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
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["to"]["name"], "callee");
}

#[test]
fn test_call_hierarchy_outgoing_calls_overlay_text_precedence() {
    let ts = create_temp_shard("overlay_prec");
    let file = ts.dir.join("src/lib.rs");
    // On disk caller makes no calls
    std::fs::write(&file, "fn callee() {}\n\nfn caller() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    // In overlay caller calls callee()
    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn callee() {}\n\nfn caller() {\n    callee();\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 663, "method": "callHierarchy/outgoingCalls",
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
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["to"]["name"], "callee");
}

#[test]
fn test_call_hierarchy_outgoing_calls_overlay_did_change() {
    let ts = create_temp_shard("overlay_change");
    let file = ts.dir.join("src/lib.rs");
    // Initially on disk caller has no calls
    std::fs::write(&file, "fn callee() {}\n\nfn caller() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn callee() {}\n\nfn caller() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let change_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": to_uri(&file), "version": 2 },
            "contentChanges": [{
                "text": "fn callee() {}\n\nfn caller() {\n    callee();\n}\n"
            }]
        }
    });
    assert!(dsp.dispatch(&change_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 664, "method": "callHierarchy/outgoingCalls",
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
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["to"]["name"], "callee");
}
