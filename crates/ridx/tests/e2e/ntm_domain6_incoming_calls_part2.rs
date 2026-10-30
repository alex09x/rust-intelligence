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

fn create_temp_shard(name: &str) -> TempShard {
    let id = INC_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_inccall_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let lib_file = dir.join("src/lib.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new(name, "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "callee".to_string(),
        path: format!("{}::callee", name),
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
        name: "caller".to_string(),
        path: format!("{}::caller", name),
        signature: None,
        file: lib_str.clone(),
        start_line: 3,
        start_col: 1,
        end_line: 3,
        end_col: 7,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 4,
        start_col: 5,
        end_line: 4,
        end_col: 11,
        target_def_id: Some(1),
        target_path: format!("{}::callee", name),
        target_file: Some(lib_str),
        target_unit: None,
        target_line: 1,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
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
fn test_call_hierarchy_incoming_calls_dispatcher_echoes_numeric_id() {
    let ts = create_temp_shard("num_id");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn callee() {}\n\nfn caller() {\n    callee();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 999444, "method": "callHierarchy/incomingCalls",
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
    assert_eq!(resp["id"], 999444);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["from"]["name"], "caller");
}

#[test]
fn test_call_hierarchy_incoming_calls_dispatcher_echoes_string_id() {
    let ts = create_temp_shard("str_id");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn callee() {}\n\nfn caller() {\n    callee();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": "inc-calls-str-1", "method": "callHierarchy/incomingCalls",
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
    assert_eq!(resp["id"], "inc-calls-str-1");
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["from"]["name"], "caller");
}

#[test]
fn test_call_hierarchy_incoming_calls_overlay_text_call_site_detection() {
    let ts = create_temp_shard("overlay_call");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn callee() {}\n\nfn caller() {\n    callee();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

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
        "jsonrpc": "2.0", "id": 628, "method": "callHierarchy/incomingCalls",
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
    assert_eq!(calls[0]["from"]["name"], "caller");
}

#[test]
fn test_call_hierarchy_incoming_calls_unknown_item_returns_empty() {
    let ts = create_temp_shard("unknown");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn callee() {}\n\nfn caller() {\n    callee();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 629, "method": "callHierarchy/incomingCalls",
        "params": {
            "item": {
                "name": "nonexistent_fn", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 50, "character": 0 }, "end": { "line": 50, "character": 10 } },
                "selectionRange": { "start": { "line": 50, "character": 0 }, "end": { "line": 50, "character": 10 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 0);

    // Positive control query on valid callee
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 6290, "method": "callHierarchy/incomingCalls",
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
fn test_call_hierarchy_incoming_calls_nonexistent_item_file_returns_empty() {
    let ts = create_temp_shard("missing_file");
    let file = ts.dir.join("src/missing.rs");
    let valid_file = ts.dir.join("src/lib.rs");
    std::fs::write(&valid_file, "fn callee() {}\n\nfn caller() {\n    callee();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 630, "method": "callHierarchy/incomingCalls",
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
    assert_eq!(calls.len(), 0);

    // Positive control query on valid file
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 6300, "method": "callHierarchy/incomingCalls",
        "params": {
            "item": {
                "name": "callee", "kind": 12, "uri": to_uri(&valid_file),
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
fn test_call_hierarchy_incoming_calls_from_ranges_contain_call_span() {
    let ts = create_temp_shard("ranges");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn callee() {}\n\nfn caller() {\n    callee();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 631, "method": "callHierarchy/incomingCalls",
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
    let from_range = &calls[0]["fromRanges"][0];
    assert_eq!(from_range["start"]["line"], 3);
    assert_eq!(from_range["start"]["character"], 4);
    assert_eq!(from_range["end"]["line"], 3);
    assert_eq!(from_range["end"]["character"], 10);
}

#[test]
fn test_call_hierarchy_incoming_calls_from_item_uri_and_range_correct() {
    let ts = create_temp_shard("item_uri");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn callee() {}\n\nfn caller() {\n    callee();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 632, "method": "callHierarchy/incomingCalls",
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
    assert_eq!(calls[0]["from"]["uri"], to_uri(&file));
    assert_eq!(calls[0]["from"]["range"]["start"]["line"], 2);
    assert_eq!(calls[0]["from"]["selectionRange"]["start"]["line"], 2);
}
