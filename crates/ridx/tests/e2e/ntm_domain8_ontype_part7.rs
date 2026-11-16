/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/onTypeFormatting (Domain 8, Part 7).
//! Tests cancellation, overlay vs disk, didClose, didChange, isolation, and editions.

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

static ONTYPE_P7_COUNTER: AtomicU64 = AtomicU64::new(1);

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
fn test_on_type_formatting_cancelled_request_returns_code_32800() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ontype_p7_canc_{}.rs", ONTYPE_P7_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let cancel_msg = json!({
        "jsonrpc": "2.0", "method": "$/cancelRequest",
        "params": { "id": 701 }
    });
    assert!(dsp.dispatch(&cancel_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 701, "method": "textDocument/onTypeFormatting",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 0 },
            "ch": ";",
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 701);
    assert_eq!(resp["error"]["code"], -32800);
}

#[test]
fn test_on_type_formatting_overlay_active_buffer_precedes_disk() {
    if !is_rustfmt_installed() {
        return;
    }
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ontype_p7_ov_{}.rs", ONTYPE_P7_COUNTER.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&file, "fn on_disk(){ let old = 1; }\n").unwrap();
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn in_overlay(){\nlet new_var=2;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 702, "method": "textDocument/onTypeFormatting",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 1 },
            "ch": "}",
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 702);

    let edits = resp["result"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
    let new_text = edits[0]["newText"].as_str().unwrap();
    assert_eq!(new_text, "fn in_overlay() {\n    let new_var = 2;\n}\n");

    let _ = std::fs::remove_file(&file);
}

/// A document the editor has not opened is not formatted from disk: its buffer may be ahead of the file.
#[test]
fn test_on_type_formatting_no_disk_fallback_when_not_in_overlay() {
    if !is_rustfmt_installed() {
        return;
    }
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ontype_p7_disk_{}.rs", ONTYPE_P7_COUNTER.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&file, "fn on_disk_only(){\nlet x=100;\n}\n").unwrap();
    let uri = to_uri(&file);

    let req = json!({
        "jsonrpc": "2.0", "id": 703, "method": "textDocument/onTypeFormatting",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 1 },
            "ch": "}",
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 703);
    assert_eq!(resp["result"], json!([]));

    let _ = std::fs::remove_file(&file);
}

/// After `didClose` the buffer is no longer the server's to know: nothing is formatted.
#[test]
fn test_on_type_formatting_after_did_close_formats_nothing() {
    if !is_rustfmt_installed() {
        return;
    }
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ontype_p7_close_{}.rs", ONTYPE_P7_COUNTER.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&file, "fn on_disk_saved(){\nlet y=200;\n}\n").unwrap();
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn transient_overlay() { 42 }\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let close_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didClose",
        "params": {
            "textDocument": { "uri": &uri }
        }
    });
    assert!(dsp.dispatch(&close_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 704, "method": "textDocument/onTypeFormatting",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 1 },
            "ch": "}",
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 704);
    assert_eq!(resp["result"], json!([]));

    let _ = std::fs::remove_file(&file);
}

#[test]
fn test_on_type_formatting_did_change_incremental_update() {
    if !is_rustfmt_installed() {
        return;
    }
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ontype_p7_change_{}.rs", ONTYPE_P7_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn first() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let change_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": &uri, "version": 2 },
            "contentChanges": [
                { "text": "fn updated(){\nlet val=10;\n}\n" }
            ]
        }
    });
    assert!(dsp.dispatch(&change_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 705, "method": "textDocument/onTypeFormatting",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 1 },
            "ch": "}",
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 705);

    let edits = resp["result"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
    let new_text = edits[0]["newText"].as_str().unwrap();
    assert_eq!(new_text, "fn updated() {\n    let val = 10;\n}\n");
}

#[test]
fn test_on_type_formatting_consecutive_requests_isolation() {
    if !is_rustfmt_installed() {
        return;
    }
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ontype_p7_isol_{}.rs", ONTYPE_P7_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn multi(){\nlet a=1;\nlet b=2;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req1 = json!({
        "jsonrpc": "2.0", "id": 706, "method": "textDocument/onTypeFormatting",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 8 },
            "ch": ";",
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req1));
    let resp1 = dsp.parse_response();
    assert_eq!(resp1["id"], 706);

    let req2 = json!({
        "jsonrpc": "2.0", "id": 707, "method": "textDocument/onTypeFormatting",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 8 },
            "ch": ";",
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req2));
    let resp2 = dsp.parse_response();
    assert_eq!(resp2["id"], 707);
}

#[test]
fn test_on_type_formatting_edition_2021_resolved() {
    if !is_rustfmt_installed() {
        return;
    }
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ontype_p7_ed_{}.rs", ONTYPE_P7_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let text = "async fn async_sample(){\nlet x=1;\n}\n";
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
        "jsonrpc": "2.0", "id": 708, "method": "textDocument/onTypeFormatting",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 1 },
            "ch": "}",
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 708);

    let edits = resp["result"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
    let new_text = edits[0]["newText"].as_str().unwrap();
    assert_eq!(new_text, "async fn async_sample() {\n    let x = 1;\n}\n");
}
