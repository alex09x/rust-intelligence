/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for formatting and rangeFormatting (Domain 8, Part 6).
//! Tests shutdown state, cancellation, consecutive request idempotence,
//! and didClose disk fallback.

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

static FMT_P6_COUNTER: AtomicU64 = AtomicU64::new(1);

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
fn test_formatting_after_shutdown_returns_code_32600() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_fmt_p6_shut_{}.rs", FMT_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let shutdown_msg = json!({
        "jsonrpc": "2.0", "id": 600, "method": "shutdown"
    });
    assert!(dsp.dispatch(&shutdown_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 601, "method": "textDocument/formatting",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 601);
    assert_eq!(resp["error"]["code"], -32600);
}

#[test]
fn test_formatting_cancelled_request_returns_code_32800() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_fmt_p6_cancel_{}.rs", FMT_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let cancel_id = json!(602);
    dsp.cancelled.lock().unwrap().insert(cancel_id.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": cancel_id, "method": "textDocument/formatting",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 602);
    assert_eq!(resp["error"]["code"], -32800);
}

#[test]
fn test_range_formatting_after_shutdown_returns_code_32600() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_fmt_p6_rshut_{}.rs", FMT_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let shutdown_msg = json!({
        "jsonrpc": "2.0", "id": 603, "method": "shutdown"
    });
    assert!(dsp.dispatch(&shutdown_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 604, "method": "textDocument/rangeFormatting",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 1, "character": 0 }
            },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 604);
    assert_eq!(resp["error"]["code"], -32600);
}

#[test]
fn test_range_formatting_cancelled_request_returns_code_32800() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_fmt_p6_rcancel_{}.rs", FMT_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let cancel_id = json!(605);
    dsp.cancelled.lock().unwrap().insert(cancel_id.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": cancel_id, "method": "textDocument/rangeFormatting",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 1, "character": 0 }
            },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 605);
    assert_eq!(resp["error"]["code"], -32800);
}

#[test]
fn test_formatting_consecutive_requests_idempotence() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_fmt_p6_consec_{}.rs", FMT_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let clean = 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req1 = json!({
        "jsonrpc": "2.0", "id": 606, "method": "textDocument/formatting",
        "params": {
            "textDocument": { "uri": &uri },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req1));
    let resp1 = dsp.parse_response();
    assert_eq!(resp1["id"], 606);
    assert_eq!(resp1["result"], json!([]));

    let req2 = json!({
        "jsonrpc": "2.0", "id": 607, "method": "textDocument/formatting",
        "params": {
            "textDocument": { "uri": &uri },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req2));
    let resp2 = dsp.parse_response();
    assert_eq!(resp2["id"], 607);
    assert_eq!(resp2["result"], json!([]));
}

#[test]
fn test_range_formatting_consecutive_requests_isolation() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_fmt_p6_rconsec_{}.rs", FMT_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn first() {}\nfn second() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req1 = json!({
        "jsonrpc": "2.0", "id": 608, "method": "textDocument/rangeFormatting",
        "params": {
            "textDocument": { "uri": &uri },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 0, "character": 13 }
            },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req1));
    let resp1 = dsp.parse_response();
    assert_eq!(resp1["id"], 608);

    let req2 = json!({
        "jsonrpc": "2.0", "id": 609, "method": "textDocument/rangeFormatting",
        "params": {
            "textDocument": { "uri": &uri },
            "range": {
                "start": { "line": 1, "character": 0 },
                "end": { "line": 1, "character": 14 }
            },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req2));
    let resp2 = dsp.parse_response();
    assert_eq!(resp2["id"], 609);
}

#[test]
fn test_formatting_did_close_falls_back_to_disk() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_fmt_p6_closefall_{}.rs", FMT_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    std::fs::write(&file, "fn main() {\n    let disk_val = 1;\n}\n").unwrap();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn overlay_val( x:i32 )->i32{ x }\n"
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
        "jsonrpc": "2.0", "id": 610, "method": "textDocument/formatting",
        "params": {
            "textDocument": { "uri": &uri },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 610);
    // Disk content is already clean, so formatting returns empty array
    assert_eq!(resp["result"], json!([]));
}
