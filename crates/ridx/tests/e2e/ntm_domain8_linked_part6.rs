/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/linkedEditingRange (Domain 8, Part 6).
//! Tests lifecycle states, cancellation, protocol version, ID types,
//! consecutive request isolation, and word boundary queries.

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

static LNK_P6_COUNTER: AtomicU64 = AtomicU64::new(1);

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
fn test_linked_editing_range_after_shutdown_returns_code_32600() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lnk_p6_shut_{}.rs", LNK_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let shutdown_msg = json!({
        "jsonrpc": "2.0", "id": 600, "method": "shutdown"
    });
    assert!(dsp.dispatch(&shutdown_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 601, "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 0 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 601);
    assert_eq!(resp["error"]["code"], -32600);
}

#[test]
fn test_linked_editing_range_cancelled_request_returns_code_32800() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lnk_p6_cancel_{}.rs", LNK_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let cancel_id = json!(602);
    dsp.cancelled.lock().unwrap().insert(cancel_id.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": cancel_id, "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 0 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 602);
    assert_eq!(resp["error"]["code"], -32800);
}

#[test]
fn test_linked_editing_range_jsonrpc_protocol_version_is_2_0() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lnk_p6_ver_{}.rs", LNK_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let val = 1;\n    val + 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 603, "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 8 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 603);
    let ranges = resp["result"]["ranges"].as_array().unwrap();
    assert_eq!(ranges.len(), 2);
}

#[test]
fn test_linked_editing_range_large_numeric_id_preserved() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lnk_p6_largeid_{}.rs", LNK_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let val = 1;\n    val + 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let large_id: u64 = 8_500_000_000_000_000_000;
    let req = json!({
        "jsonrpc": "2.0", "id": large_id, "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 8 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], large_id);
}

#[test]
fn test_linked_editing_range_string_id_preserved() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lnk_p6_strid_{}.rs", LNK_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let val = 1;\n    val + 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let str_id = "req-linked-editing-605";
    let req = json!({
        "jsonrpc": "2.0", "id": str_id, "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 8 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], str_id);
}

#[test]
fn test_linked_editing_range_negative_numeric_id_preserved() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lnk_p6_negid_{}.rs", LNK_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let val = 1;\n    val + 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let neg_id = -100;
    let req = json!({
        "jsonrpc": "2.0", "id": neg_id, "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 8 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], neg_id);
}

#[test]
fn test_linked_editing_range_consecutive_requests_isolation() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lnk_p6_consec_{}.rs", LNK_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let a = 1;\n    let b = 2;\n    a + b;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req1 = json!({
        "jsonrpc": "2.0", "id": 607, "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 8 }
        }
    });
    assert!(dsp.dispatch(&req1));
    let resp1 = dsp.parse_response();
    assert_eq!(resp1["id"], 607);
    let ranges1 = resp1["result"]["ranges"].as_array().unwrap();
    assert_eq!(ranges1.len(), 2);
    assert_eq!(ranges1[0]["start"]["line"], 1);

    let req2 = json!({
        "jsonrpc": "2.0", "id": 608, "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 8 }
        }
    });
    assert!(dsp.dispatch(&req2));
    let resp2 = dsp.parse_response();
    assert_eq!(resp2["id"], 608);
    let ranges2 = resp2["result"]["ranges"].as_array().unwrap();
    assert_eq!(ranges2.len(), 2);
    assert_eq!(ranges2[0]["start"]["line"], 2);
}

#[test]
fn test_linked_editing_range_word_boundary_start_exact_character() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lnk_p6_bstart_{}.rs", LNK_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let val = 1;\n    val + 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    // 'v' is at character index 8
    let req = json!({
        "jsonrpc": "2.0", "id": 609, "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 8 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 609);
    let ranges = resp["result"]["ranges"].as_array().unwrap();
    assert_eq!(ranges.len(), 2);
}

#[test]
fn test_linked_editing_range_word_boundary_end_exact_character() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lnk_p6_bend_{}.rs", LNK_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let val = 1;\n    val + 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    // character 12 is '=' (outside identifier and trailing space)
    let req = json!({
        "jsonrpc": "2.0", "id": 610, "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 12 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 610);
    assert!(resp["result"].is_null());
}
