/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/linkedEditingRange (Domain 8, Part 2).
//! Tests function parameters, usage sites, shadowing in nested blocks,
//! and sequential scopes.

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

static LNK_P2_COUNTER: AtomicU64 = AtomicU64::new(1);

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

        Self {
            lifecycle: lc,
            multi_root: mr,
            overlay_mgr: ov,
            cancelled: can,
            writer: Vec::new(),
        }
    }

    fn dispatch(&mut self, val: &Value) -> bool {
        self.writer.clear();
        dispatch_lsp_message(
            val.clone(),
            &mut self.writer,
            &mut self.lifecycle,
            &mut self.multi_root,
            &mut self.overlay_mgr,
            &self.cancelled,
            None,
        ).unwrap()
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
fn test_linked_editing_range_fn_single_parameter() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lnk_p2_param1_{}.rs", LNK_P2_COUNTER.fetch_add(1, Ordering::Relaxed)));
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
        "jsonrpc": "2.0", "id": 201, "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 12 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 201);

    let ranges = resp["result"]["ranges"].as_array().unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0]["start"]["line"], 0);
    assert_eq!(ranges[0]["start"]["character"], 11);
    assert_eq!(ranges[0]["end"]["character"], 16);
    assert_eq!(ranges[1]["start"]["line"], 1);
    assert_eq!(ranges[1]["start"]["character"], 4);
    assert_eq!(ranges[1]["end"]["character"], 9);
}

#[test]
fn test_linked_editing_range_fn_first_of_two_parameters() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lnk_p2_pfirst_{}.rs", LNK_P2_COUNTER.fetch_add(1, Ordering::Relaxed)));
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
        "jsonrpc": "2.0", "id": 202, "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 8 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 202);

    let ranges = resp["result"]["ranges"].as_array().unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0]["start"]["line"], 0);
    assert_eq!(ranges[0]["start"]["character"], 7);
    assert_eq!(ranges[0]["end"]["character"], 12);
    assert_eq!(ranges[1]["start"]["line"], 1);
    assert_eq!(ranges[1]["start"]["character"], 4);
    assert_eq!(ranges[1]["end"]["character"], 9);
}

#[test]
fn test_linked_editing_range_fn_second_of_two_parameters() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lnk_p2_psecond_{}.rs", LNK_P2_COUNTER.fetch_add(1, Ordering::Relaxed)));
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
        "jsonrpc": "2.0", "id": 203, "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 21 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 203);

    let ranges = resp["result"]["ranges"].as_array().unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0]["start"]["line"], 0);
    assert_eq!(ranges[0]["start"]["character"], 19);
    assert_eq!(ranges[0]["end"]["character"], 25);
    assert_eq!(ranges[1]["start"]["line"], 1);
    assert_eq!(ranges[1]["start"]["character"], 12);
    assert_eq!(ranges[1]["end"]["character"], 18);
}

#[test]
fn test_linked_editing_range_fn_parameter_usage_site() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lnk_p2_pusage_{}.rs", LNK_P2_COUNTER.fetch_add(1, Ordering::Relaxed)));
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
        "jsonrpc": "2.0", "id": 204, "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 6 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 204);

    let ranges = resp["result"]["ranges"].as_array().unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0]["start"]["line"], 0);
    assert_eq!(ranges[0]["start"]["character"], 11);
    assert_eq!(ranges[0]["end"]["character"], 16);
    assert_eq!(ranges[1]["start"]["line"], 1);
    assert_eq!(ranges[1]["start"]["character"], 4);
    assert_eq!(ranges[1]["end"]["character"], 9);
}

#[test]
fn test_linked_editing_range_nested_block_inner_shadowed_binding() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lnk_p2_shadin_{}.rs", LNK_P2_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let val = 1;\n    {\n        let val = 2;\n        val + 10;\n    }\n    val + 20;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 205, "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 3, "character": 13 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 205);

    let ranges = resp["result"]["ranges"].as_array().unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0]["start"]["line"], 3);
    assert_eq!(ranges[1]["start"]["line"], 4);
}

#[test]
fn test_linked_editing_range_nested_block_outer_shadowed_binding() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lnk_p2_shadout_{}.rs", LNK_P2_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let val = 1;\n    {\n        let val = 2;\n        val + 10;\n    }\n    val + 20;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 206, "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 9 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 206);

    let ranges = resp["result"]["ranges"].as_array().unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0]["start"]["line"], 1);
    assert_eq!(ranges[1]["start"]["line"], 6);
}

#[test]
fn test_linked_editing_range_sequential_scopes_independent_bindings() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lnk_p2_seq_{}.rs", LNK_P2_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    {\n        let a = 1;\n        a + 1;\n    }\n    {\n        let a = 2;\n        a + 2;\n    }\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 207, "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 13 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 207);

    let ranges = resp["result"]["ranges"].as_array().unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0]["start"]["line"], 2);
    assert_eq!(ranges[1]["start"]["line"], 3);
}

#[test]
fn test_linked_editing_range_binding_in_nested_closures() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lnk_p2_clos_{}.rs", LNK_P2_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let factor = 10;\n    let closure = || factor * 2;\n    factor + 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 208, "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 10 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 208);

    let ranges = resp["result"]["ranges"].as_array().unwrap();
    assert_eq!(ranges.len(), 3);
    assert_eq!(ranges[0]["start"]["line"], 1);
    assert_eq!(ranges[1]["start"]["line"], 2);
    assert_eq!(ranges[2]["start"]["line"], 3);
}
