/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/codeLens (Domain 7, Part 4).
//! Tests ordering, line shifts after incremental didChange, symbol deletion, CRLF lines, Unicode, and multi-doc isolation.

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

static LENS_P4_COUNTER: AtomicU64 = AtomicU64::new(1);

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
fn test_code_lens_multiple_functions_order_preserved() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lens_p4_order_{}.rs", LENS_P4_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn alpha() {}\nfn beta() {}\nfn gamma() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 401, "method": "textDocument/codeLens",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 401);
    let lenses = resp["result"].as_array().unwrap();
    assert_eq!(lenses.len(), 3);
    assert_eq!(lenses[0]["range"]["start"]["line"], 0);
    assert_eq!(lenses[0]["data"]["name"], "alpha");
    assert_eq!(lenses[1]["range"]["start"]["line"], 1);
    assert_eq!(lenses[1]["data"]["name"], "beta");
    assert_eq!(lenses[2]["range"]["start"]["line"], 2);
    assert_eq!(lenses[2]["data"]["name"], "gamma");
}

#[test]
fn test_code_lens_did_change_shifts_lenses_downward() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lens_p4_shift_{}.rs", LENS_P4_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn compute() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let change_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": to_uri(&file), "version": 2 },
            "contentChanges": [{ "text": "// Header line 1\n// Header line 2\n// Header line 3\nfn compute() {}\n" }]
        }
    });
    assert!(dsp.dispatch(&change_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 402, "method": "textDocument/codeLens",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 402);
    let lenses = resp["result"].as_array().unwrap();
    assert_eq!(lenses.len(), 1);
    assert_eq!(lenses[0]["range"]["start"]["line"], 3);
    assert_eq!(lenses[0]["data"]["name"], "compute");
}

#[test]
fn test_code_lens_did_change_deletes_symbol_removes_lens() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lens_p4_delete_{}.rs", LENS_P4_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn to_delete() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let change_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": to_uri(&file), "version": 2 },
            "contentChanges": [{ "text": "// replaced with comment\n" }]
        }
    });
    assert!(dsp.dispatch(&change_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 403, "method": "textDocument/codeLens",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 403);
    let lenses = resp["result"].as_array().unwrap();
    assert_eq!(lenses.len(), 0);
}

#[test]
fn test_code_lens_crlf_line_offsets_matching() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lens_p4_crlf_{}.rs", LENS_P4_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "// Header\r\nfn crlf_func() {}\r\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 404, "method": "textDocument/codeLens",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 404);
    let lenses = resp["result"].as_array().unwrap();
    assert_eq!(lenses.len(), 1);
    assert_eq!(lenses[0]["range"]["start"]["line"], 1);
    assert_eq!(lenses[0]["range"]["start"]["character"], 3);
    assert_eq!(lenses[0]["range"]["end"]["line"], 1);
    assert_eq!(lenses[0]["range"]["end"]["character"], 12);
    assert_eq!(lenses[0]["data"]["name"], "crlf_func");
}

#[test]
fn test_code_lens_unicode_identifiers_exact_offsets() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_lens_p4_uni_{}.rs", LENS_P4_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "// Σχολιασμός!\nfn calculate() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 405, "method": "textDocument/codeLens",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 405);
    let lenses = resp["result"].as_array().unwrap();
    assert_eq!(lenses.len(), 1);
    assert_eq!(lenses[0]["range"]["start"]["line"], 1);
    assert_eq!(lenses[0]["range"]["start"]["character"], 3);
    assert_eq!(lenses[0]["range"]["end"]["line"], 1);
    assert_eq!(lenses[0]["range"]["end"]["character"], 12);
    assert_eq!(lenses[0]["data"]["name"], "calculate");
}

#[test]
fn test_code_lens_two_documents_independent_lenses() {
    let mut dsp = TestDispatcher::new();
    let id = LENS_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file_a = std::env::temp_dir().join(format!("ridx_lens_p4_doca_{}.rs", id));
    let file_b = std::env::temp_dir().join(format!("ridx_lens_p4_docb_{}.rs", id));

    let open_a = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file_a), "languageId": "rust", "version": 1,
                "text": "fn single_fn() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_a));

    let open_b = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file_b), "languageId": "rust", "version": 1,
                "text": "fn first_fn() {}\nfn second_fn() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_b));

    let req_a = json!({
        "jsonrpc": "2.0", "id": 406, "method": "textDocument/codeLens",
        "params": { "textDocument": { "uri": to_uri(&file_a) } }
    });
    assert!(dsp.dispatch(&req_a));
    let resp_a = dsp.parse_response();
    assert_eq!(resp_a["id"], 406);
    let lenses_a = resp_a["result"].as_array().unwrap();
    assert_eq!(lenses_a.len(), 1);
    assert_eq!(lenses_a[0]["data"]["name"], "single_fn");

    let req_b = json!({
        "jsonrpc": "2.0", "id": 407, "method": "textDocument/codeLens",
        "params": { "textDocument": { "uri": to_uri(&file_b) } }
    });
    assert!(dsp.dispatch(&req_b));
    let resp_b = dsp.parse_response();
    assert_eq!(resp_b["id"], 407);
    let lenses_b = resp_b["result"].as_array().unwrap();
    assert_eq!(lenses_b.len(), 2);
    assert_eq!(lenses_b[0]["data"]["name"], "first_fn");
    assert_eq!(lenses_b[1]["data"]["name"], "second_fn");
}
