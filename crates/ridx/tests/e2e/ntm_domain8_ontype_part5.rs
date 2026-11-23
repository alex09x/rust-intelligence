/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/onTypeFormatting (Domain 8, Part 5).
//! Tests positions, out-of-bounds line/char, missing parameters, and notifications.

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

static ONTYPE_P5_COUNTER: AtomicU64 = AtomicU64::new(1);

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
fn test_on_type_formatting_position_at_start_of_file() {
    if !is_rustfmt_installed() {
        return;
    }
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ontype_p5_pos0_{}.rs", ONTYPE_P5_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let text = "fn foo() { let x=1; }\n";
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
        "jsonrpc": "2.0", "id": 501, "method": "textDocument/onTypeFormatting",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 0 },
            "ch": "f",
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 501);

    let edits = resp["result"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
}

#[test]
fn test_on_type_formatting_position_at_end_of_file() {
    if !is_rustfmt_installed() {
        return;
    }
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ontype_p5_posend_{}.rs", ONTYPE_P5_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let text = "fn foo() { let x=1; }\n";
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
        "jsonrpc": "2.0", "id": 502, "method": "textDocument/onTypeFormatting",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 0 },
            "ch": "\n",
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 502);

    let edits = resp["result"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
}

#[test]
fn test_on_type_formatting_position_out_of_bounds_line() {
    if !is_rustfmt_installed() {
        return;
    }
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ontype_p5_posoob_{}.rs", ONTYPE_P5_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let text = "fn foo() { let x=1; }\n";
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
        "jsonrpc": "2.0", "id": 503, "method": "textDocument/onTypeFormatting",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 9999, "character": 0 },
            "ch": ";",
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 503);
    assert_eq!(resp["error"]["code"], -32602, "a line past the end is invalid params (issue #2): {resp}");
}

#[test]
fn test_on_type_formatting_position_out_of_bounds_char() {
    if !is_rustfmt_installed() {
        return;
    }
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ontype_p5_poschar_{}.rs", ONTYPE_P5_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let text = "fn foo() { let x=1; }\n";
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
        "jsonrpc": "2.0", "id": 504, "method": "textDocument/onTypeFormatting",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 9999 },
            "ch": ";",
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 504);

    let edits = resp["result"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
}

#[test]
fn test_on_type_formatting_missing_position_defaults_gracefully() {
    if !is_rustfmt_installed() {
        return;
    }
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ontype_p5_posmiss_{}.rs", ONTYPE_P5_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let text = "fn foo() { let x=1; }\n";
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
        "jsonrpc": "2.0", "id": 505, "method": "textDocument/onTypeFormatting",
        "params": {
            "textDocument": { "uri": &uri },
            "ch": ";",
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 505);

    let edits = resp["result"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
}

#[test]
fn test_on_type_formatting_missing_ch_param_defaults_gracefully() {
    if !is_rustfmt_installed() {
        return;
    }
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ontype_p5_chmiss_{}.rs", ONTYPE_P5_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let text = "fn foo() { let x=1; }\n";
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
        "jsonrpc": "2.0", "id": 506, "method": "textDocument/onTypeFormatting",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 10 },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 506);

    let edits = resp["result"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
}

#[test]
fn test_on_type_formatting_null_id_notification_handled() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ontype_p5_nullid_{}.rs", ONTYPE_P5_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let notif = json!({
        "jsonrpc": "2.0", "method": "textDocument/onTypeFormatting",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 0 },
            "ch": ";",
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&notif));
}
