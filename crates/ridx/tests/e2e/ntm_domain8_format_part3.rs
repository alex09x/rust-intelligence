/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for formatting and rangeFormatting (Domain 8, Part 3).
//! Tests overlay vs disk fallbacks, options parsing, CRLF documents,
//! and nonexistent file handling.

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

static FMT_P3_COUNTER: AtomicU64 = AtomicU64::new(1);

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
fn test_formatting_overlay_active_buffer_precedes_disk() {
    if !is_rustfmt_installed() {
        return;
    }
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_fmt_p3_ovp_{}.rs", FMT_P3_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    std::fs::write(&file, "fn disk_clean() {\n    let clean = 1;\n}\n").unwrap();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn overlay_unformatted( a:i32,b:i32 )->i32{a+b}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 301, "method": "textDocument/formatting",
        "params": {
            "textDocument": { "uri": &uri },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 301);

    let edits = resp["result"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0]["newText"], "fn overlay_unformatted(a: i32, b: i32) -> i32 {\n    a + b\n}\n");
}

/// A document the editor has not opened is not formatted from disk: its buffer may be ahead of the file. Zed asks to
/// format a file it saves right after a rename edited it, before opening it, and the text on disk wrote the old names
/// back.
#[test]
fn test_formatting_no_disk_fallback_when_not_in_overlay() {
    if !is_rustfmt_installed() {
        return;
    }
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_fmt_p3_diskfall_{}.rs", FMT_P3_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    std::fs::write(&file, "pub fn disk_calc( x:i32,y:i32 )->i32{ x+y }\n").unwrap();

    let req = json!({
        "jsonrpc": "2.0", "id": 302, "method": "textDocument/formatting",
        "params": {
            "textDocument": { "uri": &uri },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 302);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_range_formatting_overlay_active_buffer_precedes_disk() {
    if !is_rustfmt_installed() {
        return;
    }
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_fmt_p3_rovp_{}.rs", FMT_P3_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    std::fs::write(&file, "fn disk() {}\n").unwrap();

    let text = "fn header() {}\npub fn overlay_range( a:i32 )->i32{ a }\nfn footer() {}\n";
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
        "jsonrpc": "2.0", "id": 303, "method": "textDocument/rangeFormatting",
        "params": {
            "textDocument": { "uri": &uri },
            "range": {
                "start": { "line": 1, "character": 0 },
                "end": { "line": 1, "character": 40 }
            },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 303);

    let edits = resp["result"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0]["range"]["start"]["line"], 1);
    assert_eq!(edits[0]["range"]["end"]["line"], 1);
}

#[test]
fn test_range_formatting_disk_fallback_when_not_in_overlay() {
    if !is_rustfmt_installed() {
        return;
    }
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_fmt_p3_rdiskfall_{}.rs", FMT_P3_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let text = "fn clean_head() {}\npub fn disk_range( a:i32 )->i32{ a }\nfn clean_tail() {}\n";
    std::fs::write(&file, text).unwrap();

    let req = json!({
        "jsonrpc": "2.0", "id": 304, "method": "textDocument/rangeFormatting",
        "params": {
            "textDocument": { "uri": &uri },
            "range": {
                "start": { "line": 1, "character": 0 },
                "end": { "line": 1, "character": 35 }
            },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 304);

    let edits = resp["result"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0]["range"]["start"]["line"], 1);
    assert_eq!(edits[0]["range"]["end"]["line"], 1);
}

#[test]
fn test_formatting_options_tab_size_and_spaces_parsed() {
    if !is_rustfmt_installed() {
        return;
    }
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_fmt_p3_opts_{}.rs", FMT_P3_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn compute( a:i32 )->i32{ a }\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 305, "method": "textDocument/formatting",
        "params": {
            "textDocument": { "uri": &uri },
            "options": {
                "tabSize": 2,
                "insertSpaces": true,
                "trimTrailingWhitespace": true,
                "insertFinalNewline": true
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 305);
    let edits = resp["result"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
}

#[test]
fn test_formatting_crlf_document_normalized() {
    if !is_rustfmt_installed() {
        return;
    }
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_fmt_p3_crlf_{}.rs", FMT_P3_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\r\n    let   x   =   10  ;\r\n}\r\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 306, "method": "textDocument/formatting",
        "params": {
            "textDocument": { "uri": &uri },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 306);
    let edits = resp["result"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
}

#[test]
fn test_formatting_nonexistent_file_returns_empty_array() {
    let mut dsp = TestDispatcher::new();

    let req = json!({
        "jsonrpc": "2.0", "id": 307, "method": "textDocument/formatting",
        "params": {
            "textDocument": { "uri": "file:///tmp/nonexistent/ghost_format.rs" },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 307);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_range_formatting_nonexistent_file_returns_empty_array() {
    let mut dsp = TestDispatcher::new();

    let req = json!({
        "jsonrpc": "2.0", "id": 308, "method": "textDocument/rangeFormatting",
        "params": {
            "textDocument": { "uri": "file:///tmp/nonexistent/ghost_range_format.rs" },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 10, "character": 0 }
            },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 308);
    assert_eq!(resp["result"], json!([]));
}
