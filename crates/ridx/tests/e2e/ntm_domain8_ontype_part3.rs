/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/onTypeFormatting (Domain 8, Part 3).
//! Tests chains, whitespace stripping, collapsed blank lines, syntax errors, and empty files.

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

static ONTYPE_P3_COUNTER: AtomicU64 = AtomicU64::new(1);

fn to_uri(p: &Path) -> String {
    path_to_uri(p.to_str().unwrap())
}

fn is_rustfmt_installed() -> bool {
    std::process::Command::new("rustfmt").arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
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
fn test_on_type_formatting_multiline_chain() {
    if !is_rustfmt_installed() {
        return;
    }
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ontype_p3_chain_{}.rs", ONTYPE_P3_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let text = "fn run(){\nvec![1,2,3].into_iter().map(|x| x*2).collect::<Vec<_>>()\n}\n";
    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": text
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 301, "method": "textDocument/onTypeFormatting",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 1 },
            "ch": "}",
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 301);

    let edits = resp["result"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
    let new_text = edits[0]["newText"].as_str().unwrap();
    assert_eq!(new_text, "fn run() {\n    vec![1, 2, 3].into_iter().map(|x| x * 2).collect::<Vec<_>>()\n}\n");
}

#[test]
fn test_on_type_formatting_trailing_whitespace_stripped() {
    if !is_rustfmt_installed() {
        return;
    }
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ontype_p3_ws_{}.rs", ONTYPE_P3_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let text = "fn foo() {   \n    let x = 1;   \n}   \n";
    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": text
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 302, "method": "textDocument/onTypeFormatting",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 1 },
            "ch": "}",
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 302);

    let edits = resp["result"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
    let new_text = edits[0]["newText"].as_str().unwrap();
    assert_eq!(new_text, "fn foo() {\n    let x = 1;\n}\n");
}

#[test]
fn test_on_type_formatting_consecutive_blank_lines_collapsed() {
    if !is_rustfmt_installed() {
        return;
    }
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ontype_p3_blanks_{}.rs", ONTYPE_P3_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let text = "fn a() {}\n\n\n\n\nfn b() {}\n";
    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": text
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 303, "method": "textDocument/onTypeFormatting",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 5, "character": 9 },
            "ch": "}",
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 303);

    let edits = resp["result"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
    let new_text = edits[0]["newText"].as_str().unwrap();
    assert_eq!(new_text, "fn a() {}\n\nfn b() {}\n");
}

#[test]
fn test_on_type_formatting_syntax_error_unclosed_delimiter_returns_empty() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ontype_p3_unclosed_{}.rs", ONTYPE_P3_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let text = "fn broken( { let x = 1;\n";
    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": text
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 304, "method": "textDocument/onTypeFormatting",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 23 },
            "ch": ";",
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 304);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_on_type_formatting_syntax_error_invalid_tokens_returns_empty() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ontype_p3_invalid_{}.rs", ONTYPE_P3_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let text = "fn @#$! invalid syntax {}";
    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": text
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 305, "method": "textDocument/onTypeFormatting",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 25 },
            "ch": "}",
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 305);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_on_type_formatting_empty_file_returns_empty() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ontype_p3_empty_{}.rs", ONTYPE_P3_COUNTER.fetch_add(1, Ordering::Relaxed)));
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
        "jsonrpc": "2.0", "id": 306, "method": "textDocument/onTypeFormatting",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 0 },
            "ch": "\n",
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 306);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_on_type_formatting_single_semicolon_file() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ontype_p3_semi_only_{}.rs", ONTYPE_P3_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": ";"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 307, "method": "textDocument/onTypeFormatting",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 1 },
            "ch": ";",
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 307);
    assert_eq!(resp["result"], json!([]));
}
