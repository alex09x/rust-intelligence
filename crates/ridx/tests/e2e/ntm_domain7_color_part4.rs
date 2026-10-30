/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/colorPresentation (stub) (Domain 7, Part 4).
//! Tests numeric IDs, color extremes, floating point components, extra attributes, and CRLF range offsets.

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

static COLOR_P4_COUNTER: AtomicU64 = AtomicU64::new(1);

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
fn test_color_presentation_large_numeric_id_preserved() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_color_p4_large_{}.rs", COLOR_P4_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let req = json!({
        "jsonrpc": "2.0", "id": 1122334455u64, "method": "textDocument/colorPresentation",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "color": { "red": 0.5, "green": 0.5, "blue": 0.5, "alpha": 1.0 },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 7 } }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 1122334455u64);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_color_presentation_negative_numeric_id_preserved() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_color_p4_neg_{}.rs", COLOR_P4_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let req = json!({
        "jsonrpc": "2.0", "id": -999, "method": "textDocument/colorPresentation",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "color": { "red": 0.2, "green": 0.4, "blue": 0.6, "alpha": 0.8 },
            "range": { "start": { "line": 1, "character": 4 }, "end": { "line": 1, "character": 12 } }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], -999);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_color_presentation_color_clamped_extremes_zero_and_one() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_color_p4_extremes_{}.rs", COLOR_P4_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let req_zeros = json!({
        "jsonrpc": "2.0", "id": 403, "method": "textDocument/colorPresentation",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "color": { "red": 0.0, "green": 0.0, "blue": 0.0, "alpha": 0.0 },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 5 } }
        }
    });
    assert!(dsp.dispatch(&req_zeros));
    let resp_zeros = dsp.parse_response();
    assert_eq!(resp_zeros["id"], 403);
    assert_eq!(resp_zeros["result"], json!([]));

    let req_ones = json!({
        "jsonrpc": "2.0", "id": 404, "method": "textDocument/colorPresentation",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "color": { "red": 1.0, "green": 1.0, "blue": 1.0, "alpha": 1.0 },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 5 } }
        }
    });
    assert!(dsp.dispatch(&req_ones));
    let resp_ones = dsp.parse_response();
    assert_eq!(resp_ones["id"], 404);
    assert_eq!(resp_ones["result"], json!([]));
}

#[test]
fn test_color_presentation_color_intermediate_floats() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_color_p4_floats_{}.rs", COLOR_P4_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let req = json!({
        "jsonrpc": "2.0", "id": 405, "method": "textDocument/colorPresentation",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "color": { "red": 0.1234, "green": 0.5678, "blue": 0.9012, "alpha": 0.75 },
            "range": { "start": { "line": 2, "character": 10 }, "end": { "line": 2, "character": 22 } }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 405);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_color_presentation_extra_fields_in_color_object() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_color_p4_extra_color_{}.rs", COLOR_P4_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let req = json!({
        "jsonrpc": "2.0", "id": 406, "method": "textDocument/colorPresentation",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "color": {
                "red": 0.0, "green": 1.0, "blue": 1.0, "alpha": 1.0,
                "colorSpace": "srgb", "name": "cyan"
            },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 4 } }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 406);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_color_presentation_range_with_crlf_offsets() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_color_p4_crlf_{}.rs", COLOR_P4_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "line0\r\nline1\r\nline2\r\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 407, "method": "textDocument/colorPresentation",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "color": { "red": 0.8, "green": 0.1, "blue": 0.1, "alpha": 1.0 },
            "range": { "start": { "line": 1, "character": 0 }, "end": { "line": 1, "character": 5 } }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 407);
    assert_eq!(resp["result"], json!([]));
}
