/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for formatting and rangeFormatting (Domain 8, Part 4).
//! Tests syntax errors, invalid tokens, empty files, and missing parameter fields.

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

static FMT_P4_COUNTER: AtomicU64 = AtomicU64::new(1);

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
fn test_formatting_syntax_error_unclosed_delimiter_returns_empty() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_fmt_p4_unclosed_{}.rs", FMT_P4_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn unclosed_syntax() {\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 401, "method": "textDocument/formatting",
        "params": {
            "textDocument": { "uri": &uri },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 401);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_formatting_syntax_error_invalid_tokens_returns_empty() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_fmt_p4_tokens_{}.rs", FMT_P4_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() { %%%% @@@@ $$$$ }\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 402, "method": "textDocument/formatting",
        "params": {
            "textDocument": { "uri": &uri },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 402);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_range_formatting_syntax_error_snippet_returns_empty() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_fmt_p4_rsnippet_{}.rs", FMT_P4_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn clean() {}\n%%%% invalid syntax\nfn tail() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 403, "method": "textDocument/rangeFormatting",
        "params": {
            "textDocument": { "uri": &uri },
            "range": {
                "start": { "line": 1, "character": 0 },
                "end": { "line": 1, "character": 20 }
            },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 403);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_formatting_empty_file_returns_empty() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_fmt_p4_empty_{}.rs", FMT_P4_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": ""
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 404, "method": "textDocument/formatting",
        "params": {
            "textDocument": { "uri": &uri },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 404);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_range_formatting_empty_file_returns_empty() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_fmt_p4_rempty_{}.rs", FMT_P4_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": ""
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 405, "method": "textDocument/rangeFormatting",
        "params": {
            "textDocument": { "uri": &uri },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 0, "character": 0 }
            },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 405);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_formatting_missing_text_document_returns_empty() {
    let mut dsp = TestDispatcher::new();

    let req = json!({
        "jsonrpc": "2.0", "id": 406, "method": "textDocument/formatting",
        "params": {
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 406);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_range_formatting_missing_text_document_returns_empty() {
    let mut dsp = TestDispatcher::new();

    let req = json!({
        "jsonrpc": "2.0", "id": 407, "method": "textDocument/rangeFormatting",
        "params": {
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 1, "character": 0 }
            },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 407);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_range_formatting_missing_range_param_defaults_to_all_or_empty() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_fmt_p4_norange_{}.rs", FMT_P4_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 408, "method": "textDocument/rangeFormatting",
        "params": {
            "textDocument": { "uri": &uri },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 408);
    assert_eq!(resp["result"], json!([]));
}
