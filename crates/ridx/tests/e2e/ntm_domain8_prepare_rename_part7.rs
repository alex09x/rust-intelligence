/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/prepareRename (Domain 8, Part 7).
//! Tests JSON-RPC lifecycle, cancellation, protocol conformance, and request ID variants.

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

static PREP_P7_COUNTER: AtomicU64 = AtomicU64::new(1);

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
fn test_prepare_rename_after_shutdown_returns_code_32600() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_prep_p7_shut_{}.rs", PREP_P7_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let shutdown_req = json!({ "jsonrpc": "2.0", "id": 701, "method": "shutdown" });
    assert!(dsp.dispatch(&shutdown_req));
    let shut_resp = dsp.parse_response();
    assert_eq!(shut_resp["id"], 701);

    let req = json!({
        "jsonrpc": "2.0", "id": 702, "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 0 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 702);
    assert_eq!(resp["error"]["code"], -32600);
}

#[test]
fn test_prepare_rename_cancelled_request_returns_code_32800() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_prep_p7_cancel_{}.rs", PREP_P7_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let cancel_msg = json!({
        "jsonrpc": "2.0", "method": "$/cancelRequest",
        "params": { "id": 703 }
    });
    assert!(dsp.dispatch(&cancel_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 703, "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 0 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 703);
    assert_eq!(resp["error"]["code"], -32800);
}

#[test]
fn test_prepare_rename_jsonrpc_protocol_version_is_2_0() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_prep_p7_ver_{}.rs", PREP_P7_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let valid = 1;\n    valid + 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 704, "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 1, "character": 9 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 704);
    assert_eq!(resp["result"]["placeholder"], "valid");
}

#[test]
fn test_prepare_rename_large_numeric_id_preserved() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_prep_p7_large_{}.rs", PREP_P7_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let huge = 42;\n    huge + 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 999999999, "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 1, "character": 9 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 999999999);
    assert_eq!(resp["result"]["placeholder"], "huge");
}

#[test]
fn test_prepare_rename_string_id_preserved() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_prep_p7_strid_{}.rs", PREP_P7_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let sid = 42;\n    sid + 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": "prep-str-id-12345", "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 1, "character": 9 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], "prep-str-id-12345");
    assert_eq!(resp["result"]["placeholder"], "sid");
}

#[test]
fn test_prepare_rename_negative_numeric_id_preserved() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_prep_p7_negid_{}.rs", PREP_P7_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let neg = 42;\n    neg + 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": -42, "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 1, "character": 9 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], -42);
    assert_eq!(resp["result"]["placeholder"], "neg");
}
