/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/documentColor and colorPresentation (Domain 7, Part 6).
//! Tests rapid consecutive requests, request cancellation (-32800), and post-shutdown rejection (-32600).

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

static COLOR_P6_COUNTER: AtomicU64 = AtomicU64::new(1);

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
        let mut d = Self {
            lifecycle: LifecycleManager::new(),
            multi_root: MultiRootWorkspace::new(),
            overlay_mgr: OverlayManager::new(),
            cancelled: Arc::new(Mutex::new(HashSet::new())),
            writer: Vec::new(),
        };
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
fn test_document_color_ten_rapid_consecutive_requests() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_color_p6_rapid_{}.rs", COLOR_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));

    for req_id in 601..=610 {
        let req = json!({
            "jsonrpc": "2.0", "id": req_id, "method": "textDocument/documentColor",
            "params": { "textDocument": { "uri": to_uri(&file) } }
        });
        assert!(dsp.dispatch(&req));
        let resp = dsp.parse_response();
        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["id"], req_id);
        assert_eq!(resp["result"], json!([]));
    }
}

#[test]
fn test_color_presentation_ten_rapid_consecutive_requests() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_color_p6_pres_rapid_{}.rs", COLOR_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));

    for req_id in 611..=620 {
        let req = json!({
            "jsonrpc": "2.0", "id": req_id, "method": "textDocument/colorPresentation",
            "params": {
                "textDocument": { "uri": to_uri(&file) },
                "color": { "red": 0.25, "green": 0.5, "blue": 0.75, "alpha": 1.0 },
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 10 } }
            }
        });
        assert!(dsp.dispatch(&req));
        let resp = dsp.parse_response();
        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["id"], req_id);
        assert_eq!(resp["result"], json!([]));
    }
}

#[test]
fn test_document_color_cancelled_request_returns_code_32800() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_color_p6_cancel_{}.rs", COLOR_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));

    dsp.cancelled.lock().unwrap().insert(json!(621));

    let req = json!({
        "jsonrpc": "2.0", "id": 621, "method": "textDocument/documentColor",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 621);
    assert_eq!(resp["error"]["code"], -32800);
    assert_eq!(resp["error"]["message"], "Request cancelled");
}

#[test]
fn test_color_presentation_cancelled_request_returns_code_32800() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_color_p6_pres_cancel_{}.rs", COLOR_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));

    dsp.cancelled.lock().unwrap().insert(json!(622));

    let req = json!({
        "jsonrpc": "2.0", "id": 622, "method": "textDocument/colorPresentation",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "color": { "red": 1.0, "green": 0.0, "blue": 0.0, "alpha": 1.0 },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 4 } }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 622);
    assert_eq!(resp["error"]["code"], -32800);
    assert_eq!(resp["error"]["message"], "Request cancelled");
}

#[test]
fn test_document_color_after_shutdown_returns_code_32600() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_color_p6_shut_{}.rs", COLOR_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let shutdown_req = json!({ "jsonrpc": "2.0", "id": 623, "method": "shutdown" });
    assert!(dsp.dispatch(&shutdown_req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 623);
    assert_eq!(resp["result"], Value::Null);

    let req = json!({
        "jsonrpc": "2.0", "id": 624, "method": "textDocument/documentColor",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 624);
    assert_eq!(resp["error"]["code"], -32600);
    assert_eq!(resp["error"]["message"], "Server is shutting down");
}

#[test]
fn test_color_presentation_after_shutdown_returns_code_32600() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_color_p6_pres_shut_{}.rs", COLOR_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let shutdown_req = json!({ "jsonrpc": "2.0", "id": 625, "method": "shutdown" });
    assert!(dsp.dispatch(&shutdown_req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 625);
    assert_eq!(resp["result"], Value::Null);

    let req = json!({
        "jsonrpc": "2.0", "id": 626, "method": "textDocument/colorPresentation",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "color": { "red": 0.0, "green": 0.0, "blue": 1.0, "alpha": 1.0 },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 6 } }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 626);
    assert_eq!(resp["error"]["code"], -32600);
    assert_eq!(resp["error"]["message"], "Server is shutting down");
}
