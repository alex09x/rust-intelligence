/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/documentColor and colorPresentation workflows (Domain 7, Part 5).
//! Tests sequence workflows, multiple open documents, missing parameters, and empty objects.

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

static COLOR_P5_COUNTER: AtomicU64 = AtomicU64::new(1);

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
fn test_color_workflow_document_color_then_presentation() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_color_p5_flow_{}.rs", COLOR_P5_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": { "uri": to_uri(&file), "languageId": "rust", "version": 1, "text": "let c = \"#aabbcc\";\n" }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let doc_color_req = json!({
        "jsonrpc": "2.0", "id": 501, "method": "textDocument/documentColor",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&doc_color_req));
    let resp1 = dsp.parse_response();
    assert_eq!(resp1["id"], 501);
    assert_eq!(resp1["result"], json!([]));

    let pres_req = json!({
        "jsonrpc": "2.0", "id": 502, "method": "textDocument/colorPresentation",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "color": { "red": 0.667, "green": 0.733, "blue": 0.8, "alpha": 1.0 },
            "range": { "start": { "line": 0, "character": 8 }, "end": { "line": 0, "character": 17 } }
        }
    });
    assert!(dsp.dispatch(&pres_req));
    let resp2 = dsp.parse_response();
    assert_eq!(resp2["id"], 502);
    assert_eq!(resp2["result"], json!([]));
}

#[test]
fn test_color_workflow_multiple_documents_independent_queries() {
    let mut dsp = TestDispatcher::new();
    let id = COLOR_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file_a = std::env::temp_dir().join(format!("ridx_color_p5_a_{}.rs", id));
    let file_b = std::env::temp_dir().join(format!("ridx_color_p5_b_{}.rs", id));

    let open_a = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": { "uri": to_uri(&file_a), "languageId": "rust", "version": 1, "text": "const A: u32 = 1;\n" }
        }
    });
    assert!(dsp.dispatch(&open_a));

    let open_b = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": { "uri": to_uri(&file_b), "languageId": "rust", "version": 1, "text": "const B: u32 = 2;\n" }
        }
    });
    assert!(dsp.dispatch(&open_b));

    let req_a = json!({
        "jsonrpc": "2.0", "id": 503, "method": "textDocument/documentColor",
        "params": { "textDocument": { "uri": to_uri(&file_a) } }
    });
    assert!(dsp.dispatch(&req_a));
    let resp_a = dsp.parse_response();
    assert_eq!(resp_a["id"], 503);
    assert_eq!(resp_a["result"], json!([]));

    let req_b = json!({
        "jsonrpc": "2.0", "id": 504, "method": "textDocument/documentColor",
        "params": { "textDocument": { "uri": to_uri(&file_b) } }
    });
    assert!(dsp.dispatch(&req_b));
    let resp_b = dsp.parse_response();
    assert_eq!(resp_b["id"], 504);
    assert_eq!(resp_b["result"], json!([]));
}

#[test]
fn test_color_presentation_missing_params_completely() {
    let mut dsp = TestDispatcher::new();

    let req = json!({
        "jsonrpc": "2.0", "id": 505, "method": "textDocument/colorPresentation"
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 505);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_color_presentation_null_params() {
    let mut dsp = TestDispatcher::new();

    let req = json!({
        "jsonrpc": "2.0", "id": 506, "method": "textDocument/colorPresentation",
        "params": Value::Null
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 506);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_color_presentation_empty_params_object() {
    let mut dsp = TestDispatcher::new();

    let req = json!({
        "jsonrpc": "2.0", "id": 507, "method": "textDocument/colorPresentation",
        "params": {}
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 507);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_document_color_empty_params_object() {
    let mut dsp = TestDispatcher::new();

    let req = json!({
        "jsonrpc": "2.0", "id": 508, "method": "textDocument/documentColor",
        "params": {}
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 508);
    assert_eq!(resp["result"], json!([]));
}
