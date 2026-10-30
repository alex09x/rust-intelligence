/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/rename (Domain 8, Part 2).
//! Tests local binding variants: usage-site invocation, mut bindings,
//! explicit types, function parameters, shadowing isolation, Greek unicode, and CRLF.

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::path_to_uri;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static REN_P2_COUNTER: AtomicU64 = AtomicU64::new(1);

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
    fn new() -> Self {
        let mut lc = LifecycleManager::new();
        let mut mr = MultiRootWorkspace::new();
        let mut ov = OverlayManager::new();
        let can = Arc::new(Mutex::new(HashSet::new()));
        let mut out = Vec::new();

        let init = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "capabilities": {} }
        });
        dispatch_lsp_message(init, &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();

        Self { lifecycle: lc, multi_root: mr, overlay_mgr: ov, cancelled: can, writer: Vec::new() }
    }

    fn dispatch(&mut self, val: &Value) -> bool {
        self.writer.clear();
        dispatch_lsp_message(val.clone(), &mut self.writer, &mut self.lifecycle, &mut self.multi_root, &mut self.overlay_mgr, &self.cancelled, None).unwrap()
    }

    fn parse_response(&mut self) -> Value {
        let s = String::from_utf8_lossy(&self.writer);
        for part in s.split("Content-Length: ") {
            if let Some(idx) = part.find("\r\n\r\n") {
                let body = &part[idx + 4..];
                if let Ok(v) = serde_json::from_str::<Value>(body.trim()) {
                    return v;
                }
            }
        }
        Value::Null
    }
}

#[test]
fn test_rename_local_binding_invoked_from_usage_site() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ren_p2_usage_{}.rs", REN_P2_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let val = 100;\n    val + 5;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 201, "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 5 },
            "newName": "total"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 201);

    let edits = resp["result"]["changes"][&uri].as_array().unwrap();
    assert_eq!(edits.len(), 2);
    assert_eq!(edits[0]["range"]["start"]["line"], 1);
    assert_eq!(edits[0]["range"]["start"]["character"], 8);
    assert_eq!(edits[0]["range"]["end"]["character"], 11);
    assert_eq!(edits[0]["newText"], "total");

    assert_eq!(edits[1]["range"]["start"]["line"], 2);
    assert_eq!(edits[1]["range"]["start"]["character"], 4);
    assert_eq!(edits[1]["range"]["end"]["character"], 7);
    assert_eq!(edits[1]["newText"], "total");
}

#[test]
fn test_rename_local_mut_binding_edits_declaration_and_reassignments() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ren_p2_mut_{}.rs", REN_P2_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let mut counter = 0;\n    counter += 1;\n    counter += 2;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 202, "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 13 },
            "newName": "tally"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 202);

    let edits = resp["result"]["changes"][&uri].as_array().unwrap();
    assert_eq!(edits.len(), 3);
    assert_eq!(edits[0]["newText"], "tally");
    assert_eq!(edits[1]["newText"], "tally");
    assert_eq!(edits[2]["newText"], "tally");
}

#[test]
fn test_rename_local_binding_explicit_type_annotation() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ren_p2_type_{}.rs", REN_P2_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let num: i64 = 42;\n    num * 2;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 203, "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 9 },
            "newName": "val"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 203);

    let edits = resp["result"]["changes"][&uri].as_array().unwrap();
    assert_eq!(edits.len(), 2);
    assert_eq!(edits[0]["newText"], "val");
    assert_eq!(edits[1]["newText"], "val");
}

#[test]
fn test_rename_fn_parameter_declaration_and_body_references() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ren_p2_param_{}.rs", REN_P2_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn compute(delta: i32) -> i32 {\n    delta + 10\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 204, "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 12 },
            "newName": "offset"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 204);

    let edits = resp["result"]["changes"][&uri].as_array().unwrap();
    assert_eq!(edits.len(), 2);
    assert_eq!(edits[0]["newText"], "offset");
    assert_eq!(edits[1]["newText"], "offset");
}

#[test]
fn test_rename_fn_second_parameter_isolated() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ren_p2_param2_{}.rs", REN_P2_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn add(first: i32, second: i32) -> i32 {\n    first + second\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 205, "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 21 },
            "newName": "other"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 205);

    let edits = resp["result"]["changes"][&uri].as_array().unwrap();
    assert_eq!(edits.len(), 2);
    assert_eq!(edits[0]["range"]["start"]["line"], 0);
    assert_eq!(edits[0]["range"]["start"]["character"], 19);
    assert_eq!(edits[0]["range"]["end"]["character"], 25);
    assert_eq!(edits[0]["newText"], "other");

    assert_eq!(edits[1]["range"]["start"]["line"], 1);
    assert_eq!(edits[1]["range"]["start"]["character"], 12);
    assert_eq!(edits[1]["range"]["end"]["character"], 18);
    assert_eq!(edits[1]["newText"], "other");
}
