/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/codeLens and codeLens/resolve (Domain 7, Part 6).
//! Tests 64-bit and negative request IDs, and request cancellation handling (-32800).

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

static LENS_P6_COUNTER: AtomicU64 = AtomicU64::new(1);

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
fn test_code_lens_large_numeric_id_preserved() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lens_p6_large_{}.rs", LENS_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let req = json!({
        "jsonrpc": "2.0", "id": 8899001122u64, "method": "textDocument/codeLens",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 8899001122u64);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_code_lens_negative_numeric_id_preserved() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lens_p6_neg_{}.rs", LENS_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let req = json!({
        "jsonrpc": "2.0", "id": -777, "method": "textDocument/codeLens",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], -777);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_code_lens_resolve_large_numeric_id_preserved() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lens_p6_res_large_{}.rs", LENS_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let req = json!({
        "jsonrpc": "2.0", "id": 7766554433u64, "method": "codeLens/resolve",
        "params": {
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 5 } },
            "data": { "uri": to_uri(&file), "line": 1, "character": 1, "name": "foo" }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 7766554433u64);
    assert_eq!(resp["result"]["command"]["command"], "ridx.showReferences");
}

#[test]
fn test_code_lens_resolve_negative_numeric_id_preserved() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lens_p6_res_neg_{}.rs", LENS_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let req = json!({
        "jsonrpc": "2.0", "id": -888, "method": "codeLens/resolve",
        "params": {
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 5 } },
            "data": { "uri": to_uri(&file), "line": 1, "character": 1, "name": "bar" }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], -888);
    assert_eq!(resp["result"]["command"]["command"], "ridx.showReferences");
}

#[test]
fn test_code_lens_cancelled_request_returns_code_32800() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lens_p6_cancel_{}.rs", LENS_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));

    dsp.cancelled.lock().unwrap().insert(json!(605));

    let req = json!({
        "jsonrpc": "2.0", "id": 605, "method": "textDocument/codeLens",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 605);
    assert_eq!(resp["error"]["code"], -32800);
    assert_eq!(resp["error"]["message"], "Request cancelled");
}

#[test]
fn test_code_lens_resolve_cancelled_request_returns_code_32800() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lens_p6_res_cancel_{}.rs", LENS_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));

    dsp.cancelled.lock().unwrap().insert(json!(606));

    let req = json!({
        "jsonrpc": "2.0", "id": 606, "method": "codeLens/resolve",
        "params": {
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 5 } },
            "data": { "uri": to_uri(&file), "line": 1, "character": 1, "name": "cancel_target" }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 606);
    assert_eq!(resp["error"]["code"], -32800);
    assert_eq!(resp["error"]["message"], "Request cancelled");
}
