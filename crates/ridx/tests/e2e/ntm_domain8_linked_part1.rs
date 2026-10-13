/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/linkedEditingRange (Domain 8, Part 1).
//! Tests local binding declarations, usage-site queries, multiple usages, mut bindings,
//! explicit types, and variable name lengths.

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

static LNK_P1_COUNTER: AtomicU64 = AtomicU64::new(1);

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
fn test_linked_editing_range_local_binding_declaration_site() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lnk_p1_decl_{}.rs", LNK_P1_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let target = 1;\n    target + 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 101, "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 9 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 101);

    let ranges = resp["result"]["ranges"].as_array().unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0]["start"]["line"], 1);
    assert_eq!(ranges[0]["start"]["character"], 8);
    assert_eq!(ranges[0]["end"]["line"], 1);
    assert_eq!(ranges[0]["end"]["character"], 14);

    assert_eq!(ranges[1]["start"]["line"], 2);
    assert_eq!(ranges[1]["start"]["character"], 4);
    assert_eq!(ranges[1]["end"]["line"], 2);
    assert_eq!(ranges[1]["end"]["character"], 10);
}

#[test]
fn test_linked_editing_range_local_binding_first_usage_site() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lnk_p1_u1_{}.rs", LNK_P1_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let target = 1;\n    target + 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 102, "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 6 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 102);

    let ranges = resp["result"]["ranges"].as_array().unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0]["start"]["line"], 1);
    assert_eq!(ranges[0]["start"]["character"], 8);
    assert_eq!(ranges[0]["end"]["character"], 14);
    assert_eq!(ranges[1]["start"]["line"], 2);
    assert_eq!(ranges[1]["start"]["character"], 4);
    assert_eq!(ranges[1]["end"]["character"], 10);
}

#[test]
fn test_linked_editing_range_local_binding_second_usage_site() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lnk_p1_u2_{}.rs", LNK_P1_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let target = 1;\n    target + 1;\n    target + 2;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 103, "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 3, "character": 6 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 103);

    let ranges = resp["result"]["ranges"].as_array().unwrap();
    assert_eq!(ranges.len(), 3);
    assert_eq!(ranges[0]["start"]["line"], 1);
    assert_eq!(ranges[1]["start"]["line"], 2);
    assert_eq!(ranges[2]["start"]["line"], 3);
}

#[test]
fn test_linked_editing_range_local_binding_three_usages_all_included() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lnk_p1_u3_{}.rs", LNK_P1_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let var = 10;\n    let a = var + 1;\n    let b = var + 2;\n    let c = var + 3;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 104, "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 9 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 104);

    let ranges = resp["result"]["ranges"].as_array().unwrap();
    assert_eq!(ranges.len(), 4);
    assert_eq!(ranges[0]["start"]["line"], 1);
    assert_eq!(ranges[1]["start"]["line"], 2);
    assert_eq!(ranges[2]["start"]["line"], 3);
    assert_eq!(ranges[3]["start"]["line"], 4);
}

#[test]
fn test_linked_editing_range_mut_binding_declaration_and_reassignments() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lnk_p1_mut_{}.rs", LNK_P1_COUNTER.fetch_add(1, Ordering::Relaxed)));
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
        "jsonrpc": "2.0", "id": 105, "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 14 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 105);

    let ranges = resp["result"]["ranges"].as_array().unwrap();
    assert_eq!(ranges.len(), 3);
    assert_eq!(ranges[0]["start"]["line"], 1);
    assert_eq!(ranges[0]["start"]["character"], 12);
    assert_eq!(ranges[0]["end"]["character"], 19);
    assert_eq!(ranges[1]["start"]["line"], 2);
    assert_eq!(ranges[1]["start"]["character"], 4);
    assert_eq!(ranges[1]["end"]["character"], 11);
    assert_eq!(ranges[2]["start"]["line"], 3);
    assert_eq!(ranges[2]["start"]["character"], 4);
    assert_eq!(ranges[2]["end"]["character"], 11);
}

#[test]
fn test_linked_editing_range_explicit_type_annotation() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lnk_p1_type_{}.rs", LNK_P1_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let val: i64 = 42;\n    val * 2;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 106, "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 9 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 106);

    let ranges = resp["result"]["ranges"].as_array().unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0]["start"]["character"], 8);
    assert_eq!(ranges[0]["end"]["character"], 11);
    assert_eq!(ranges[1]["start"]["character"], 4);
    assert_eq!(ranges[1]["end"]["character"], 7);
}

#[test]
fn test_linked_editing_range_single_letter_binding_name() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lnk_p1_single_{}.rs", LNK_P1_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let x = 10;\n    x + 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 107, "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 8 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 107);

    let ranges = resp["result"]["ranges"].as_array().unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0]["start"]["character"], 8);
    assert_eq!(ranges[0]["end"]["character"], 9);
    assert_eq!(ranges[1]["start"]["character"], 4);
    assert_eq!(ranges[1]["end"]["character"], 5);
}

#[test]
fn test_linked_editing_range_long_binding_name() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lnk_p1_long_{}.rs", LNK_P1_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let very_long_descriptive_variable_name = 1;\n    very_long_descriptive_variable_name + 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 108, "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 12 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 108);

    let ranges = resp["result"]["ranges"].as_array().unwrap();
    assert_eq!(ranges.len(), 2);
    let len = "very_long_descriptive_variable_name".len() as u64;
    assert_eq!(ranges[0]["start"]["character"], 8);
    assert_eq!(ranges[0]["end"]["character"], 8 + len);
    assert_eq!(ranges[1]["start"]["character"], 4);
    assert_eq!(ranges[1]["end"]["character"], 4 + len);
}
