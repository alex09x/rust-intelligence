/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/documentColor and colorPresentation (Domain 7, Part 7).
//! Tests JSON-RPC invariants, alphanumeric/special-character string IDs, reopen cycles, and zero-width ranges.

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

static COLOR_P7_COUNTER: AtomicU64 = AtomicU64::new(1);

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
fn test_document_color_jsonrpc_protocol_version_is_2_0() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_color_p7_v20_{}.rs", COLOR_P7_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let req = json!({
        "jsonrpc": "2.0", "id": "protocol-test-id", "method": "textDocument/documentColor",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], "protocol-test-id");
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_color_presentation_jsonrpc_protocol_version_is_2_0() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_color_p7_pres_v20_{}.rs", COLOR_P7_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let req = json!({
        "jsonrpc": "2.0", "id": "pres-v20-id", "method": "textDocument/colorPresentation",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "color": { "red": 0.5, "green": 0.5, "blue": 0.5, "alpha": 1.0 },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 5 } }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], "pres-v20-id");
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_document_color_string_id_with_special_characters() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_color_p7_str_id_{}.rs", COLOR_P7_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let special_id = "color-req-uuid-#1234/test:foo";
    let req = json!({
        "jsonrpc": "2.0", "id": special_id, "method": "textDocument/documentColor",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], special_id);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_color_presentation_string_id_with_special_characters() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_color_p7_pres_str_id_{}.rs", COLOR_P7_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let special_id = "presentation-req-uuid-#5678/test:bar";
    let req = json!({
        "jsonrpc": "2.0", "id": special_id, "method": "textDocument/colorPresentation",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "color": { "red": 0.1, "green": 0.2, "blue": 0.3, "alpha": 1.0 },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 3 } }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], special_id);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_document_color_did_close_then_reopen_returns_empty() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_color_p7_reopen_{}.rs", COLOR_P7_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let open1 = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": { "uri": to_uri(&file), "languageId": "rust", "version": 1, "text": "let c1 = 1;\n" }
        }
    });
    assert!(dsp.dispatch(&open1));

    let close_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didClose",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&close_msg));

    let open2 = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": { "uri": to_uri(&file), "languageId": "rust", "version": 2, "text": "let c2 = 2;\n" }
        }
    });
    assert!(dsp.dispatch(&open2));

    let req = json!({
        "jsonrpc": "2.0", "id": 705, "method": "textDocument/documentColor",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 705);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_color_presentation_zero_width_character_offsets() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_color_p7_zerowidth_{}.rs", COLOR_P7_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let req = json!({
        "jsonrpc": "2.0", "id": 706, "method": "textDocument/colorPresentation",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "color": { "red": 0.0, "green": 0.0, "blue": 0.0, "alpha": 1.0 },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 706);
    assert_eq!(resp["result"], json!([]));
}
