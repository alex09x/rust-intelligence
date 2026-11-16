/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/prepareRename (Domain 8, Part 6).
//! Tests Greek multibyte prefix columns, CRLF line endings, unsupported pattern rejection, comments, and query isolation.

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

static PREP_P6_COUNTER: AtomicU64 = AtomicU64::new(1);

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
fn test_prepare_rename_greek_multibyte_prefix_columns() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_prep_p6_greek_{}.rs", PREP_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "// Γειάσα\nfn main() {\n    let value = 42;\n    value + 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 601, "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 2, "character": 9 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 601);
    assert_eq!(resp["result"]["placeholder"], "value");
    assert_eq!(resp["result"]["range"]["start"]["line"], 2);
    assert_eq!(resp["result"]["range"]["start"]["character"], 8);
    assert_eq!(resp["result"]["range"]["end"]["line"], 2);
    assert_eq!(resp["result"]["range"]["end"]["character"], 13);
}

#[test]
fn test_prepare_rename_crlf_line_endings_preserved() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_prep_p6_crlf_{}.rs", PREP_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn main() {\r\n    let crlf_var = 100;\r\n    crlf_var + 1;\r\n}\r\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 602, "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 1, "character": 9 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 602);
    assert_eq!(resp["result"]["placeholder"], "crlf_var");
    assert_eq!(resp["result"]["range"]["start"]["line"], 1);
    assert_eq!(resp["result"]["range"]["start"]["character"], 8);
    assert_eq!(resp["result"]["range"]["end"]["line"], 1);
    assert_eq!(resp["result"]["range"]["end"]["character"], 16);
}

#[test]
fn test_prepare_rename_while_let_binding_is_renamable() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_prep_p6_while_{}.rs", PREP_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn loop_queue(mut queue: Option<i32>) {\n    while let Some(val) = queue {\n        val + 1;\n    }\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 603, "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 1, "character": 20 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 603);
    // A `while let` binding is a local the overlay knows, as rust-analyzer renames it.
    assert_eq!(resp["result"]["placeholder"], "val");
}

#[test]
fn test_prepare_rename_if_let_binding_is_renamable() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_prep_p6_iflet_{}.rs", PREP_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn check(opt: Option<i32>) {\n    if let Some(res) = opt {\n        res + 1;\n    }\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 604, "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 1, "character": 17 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 604);
    assert_eq!(resp["result"]["placeholder"], "res");
}

#[test]
fn test_prepare_rename_comment_token_returns_error_32803() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_prep_p6_comment_{}.rs", PREP_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "// This is an explanatory comment\nfn main() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 605, "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 5 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 605);
    assert_eq!(resp["error"]["code"], -32803);
    assert_eq!(resp["error"]["message"], "Cannot prepare rename: no identifier found at this position");
}

#[test]
fn test_prepare_rename_consecutive_queries_isolation() {
    let mut dsp = TestDispatcher::new();
    let file1 = std::env::temp_dir().join(format!("ridx_prep_p6_iso1_{}.rs", PREP_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let file2 = std::env::temp_dir().join(format!("ridx_prep_p6_iso2_{}.rs", PREP_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let open_msg1 = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file1), "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let first = 1;\n    first + 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg1));

    let open_msg2 = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file2), "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let second = 2;\n    second + 2;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg2));

    let req1 = json!({
        "jsonrpc": "2.0", "id": 606, "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": to_uri(&file1) },
            "position": { "line": 1, "character": 9 }
        }
    });
    assert!(dsp.dispatch(&req1));
    let resp1 = dsp.parse_response();
    assert_eq!(resp1["id"], 606);
    assert_eq!(resp1["result"]["placeholder"], "first");

    let req2 = json!({
        "jsonrpc": "2.0", "id": 607, "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": to_uri(&file2) },
            "position": { "line": 1, "character": 9 }
        }
    });
    assert!(dsp.dispatch(&req2));
    let resp2 = dsp.parse_response();
    assert_eq!(resp2["id"], 607);
    assert_eq!(resp2["result"]["placeholder"], "second");
}
