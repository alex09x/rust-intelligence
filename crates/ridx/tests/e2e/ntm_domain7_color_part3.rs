/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/documentColor (stub) (Domain 7, Part 3).
//! Tests edge parameter variations, large IDs, unexpected payload fields, and lifecycle modifications.

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

static COLOR_P3_COUNTER: AtomicU64 = AtomicU64::new(1);

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
fn test_document_color_large_numeric_id_preserved() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_color_p3_large_id_{}.rs", COLOR_P3_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let req = json!({
        "jsonrpc": "2.0", "id": 9876543210u64, "method": "textDocument/documentColor",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 9876543210u64);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_document_color_negative_numeric_id_preserved() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_color_p3_neg_id_{}.rs", COLOR_P3_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let req = json!({
        "jsonrpc": "2.0", "id": -42, "method": "textDocument/documentColor",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], -42);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_document_color_null_params_handled() {
    let mut dsp = TestDispatcher::new();

    let req = json!({
        "jsonrpc": "2.0", "id": 303, "method": "textDocument/documentColor",
        "params": Value::Null
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 303);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_document_color_extra_unexpected_param_fields() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_color_p3_extra_{}.rs", COLOR_P3_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let req = json!({
        "jsonrpc": "2.0", "id": 304, "method": "textDocument/documentColor",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "extra": "unexpected_payload",
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 1, "character": 0 } }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 304);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_document_color_multiple_consecutive_did_change_events() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_color_p3_changes_{}.rs", COLOR_P3_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": { "uri": to_uri(&file), "languageId": "rust", "version": 1, "text": "const RED: &str = \"#ff0000\";\n" }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let change1 = json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": to_uri(&file), "version": 2 },
            "contentChanges": [{ "text": "const GREEN: &str = \"#00ff00\";\n" }]
        }
    });
    assert!(dsp.dispatch(&change1));

    let change2 = json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": to_uri(&file), "version": 3 },
            "contentChanges": [{ "text": "const BLUE: &str = \"rgb(0, 0, 255)\";\n" }]
        }
    });
    assert!(dsp.dispatch(&change2));

    let req = json!({
        "jsonrpc": "2.0", "id": 305, "method": "textDocument/documentColor",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 305);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_document_color_did_save_notification_cycle() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_color_p3_save_{}.rs", COLOR_P3_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": { "uri": to_uri(&file), "languageId": "rust", "version": 1, "text": "const WHITE: &str = \"#ffffff\";\n" }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let save_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didSave",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&save_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 306, "method": "textDocument/documentColor",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 306);
    assert_eq!(resp["result"], json!([]));
}
