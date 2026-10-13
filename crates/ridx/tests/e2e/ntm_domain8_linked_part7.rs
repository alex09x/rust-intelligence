/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/linkedEditingRange (Domain 8, Part 7).
//! Tests horizontal shifts, didClose cleanup, reopen document, and multi-document isolation.

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

static LNK_P7_COUNTER: AtomicU64 = AtomicU64::new(1);

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
fn test_linked_editing_range_did_change_horizontal_shift() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lnk_p7_horshift_{}.rs", LNK_P7_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let foo = 1;\n    foo + 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let change_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": &uri, "version": 2 },
            "contentChanges": [{
                "text": "fn main() {\n        let foo = 1;\n        foo + 1;\n}\n"
            }]
        }
    });
    assert!(dsp.dispatch(&change_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 305, "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 13 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 305);

    let ranges = resp["result"]["ranges"].as_array().unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0]["start"]["character"], 12);
    assert_eq!(ranges[0]["end"]["character"], 15);
    assert_eq!(ranges[1]["start"]["character"], 8);
    assert_eq!(ranges[1]["end"]["character"], 11);
}

#[test]
fn test_linked_editing_range_did_close_document_clears_overlay_returns_null() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lnk_p7_close_{}.rs", LNK_P7_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let foo = 1;\n    foo + 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let close_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didClose",
        "params": {
            "textDocument": { "uri": &uri }
        }
    });
    assert!(dsp.dispatch(&close_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 306, "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 9 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 306);
    assert!(resp["result"].is_null());
}

#[test]
fn test_linked_editing_range_reopen_document_restores_capabilities() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lnk_p7_reopen_{}.rs", LNK_P7_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg1 = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let foo = 1;\n    foo + 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg1));

    let close_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didClose",
        "params": {
            "textDocument": { "uri": &uri }
        }
    });
    assert!(dsp.dispatch(&close_msg));

    let open_msg2 = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 2,
                "text": "fn main() {\n    let restored = 10;\n    restored + 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg2));

    let req = json!({
        "jsonrpc": "2.0", "id": 307, "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 9 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 307);

    let ranges = resp["result"]["ranges"].as_array().unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0]["start"]["character"], 8);
    assert_eq!(ranges[0]["end"]["character"], 16);
}

#[test]
fn test_linked_editing_range_multiple_documents_isolated() {
    let mut dsp = TestDispatcher::new();
    let file1 = std::env::temp_dir().join(format!("ridx_lnk_p7_doc1_{}.rs", LNK_P7_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let file2 = std::env::temp_dir().join(format!("ridx_lnk_p7_doc2_{}.rs", LNK_P7_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri1 = to_uri(&file1);
    let uri2 = to_uri(&file2);

    let open1 = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri1, "languageId": "rust", "version": 1,
                "text": "fn f1() {\n    let common = 1;\n    common + 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open1));

    let open2 = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri2, "languageId": "rust", "version": 1,
                "text": "fn f2() {\n    let common = 2;\n    common + 2;\n    common + 3;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open2));

    let req1 = json!({
        "jsonrpc": "2.0", "id": 308, "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": &uri1 },
            "position": { "line": 1, "character": 9 }
        }
    });
    assert!(dsp.dispatch(&req1));
    let resp1 = dsp.parse_response();
    assert_eq!(resp1["id"], 308);
    let ranges1 = resp1["result"]["ranges"].as_array().unwrap();
    assert_eq!(ranges1.len(), 2);

    let req2 = json!({
        "jsonrpc": "2.0", "id": 309, "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": &uri2 },
            "position": { "line": 1, "character": 9 }
        }
    });
    assert!(dsp.dispatch(&req2));
    let resp2 = dsp.parse_response();
    assert_eq!(resp2["id"], 309);
    let ranges2 = resp2["result"]["ranges"].as_array().unwrap();
    assert_eq!(ranges2.len(), 3);
}
