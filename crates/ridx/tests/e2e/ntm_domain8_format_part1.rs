/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/formatting (Domain 8, Part 1).
//! Tests full document formatting, idempotence, spacing, trailing whitespace,
//! consecutive newlines, structs, enums, match expressions, and iterator chains.

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

static FMT_P1_COUNTER: AtomicU64 = AtomicU64::new(1);

fn to_uri(p: &Path) -> String {
    path_to_uri(p.to_str().unwrap())
}

fn is_rustfmt_installed() -> bool {
    std::process::Command::new("rustfmt")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
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

        Self {
            lifecycle: lc,
            multi_root: mr,
            overlay_mgr: ov,
            cancelled: can,
            writer: Vec::new(),
        }
    }

    fn dispatch(&mut self, val: &Value) -> bool {
        self.writer.clear();
        dispatch_lsp_message(
            val.clone(),
            &mut self.writer,
            &mut self.lifecycle,
            &mut self.multi_root,
            &mut self.overlay_mgr,
            &self.cancelled,
            None,
        ).unwrap()
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
fn test_formatting_unformatted_spaces_around_params() {
    if !is_rustfmt_installed() {
        return;
    }
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_fmt_p1_params_{}.rs", FMT_P1_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "pub fn add_vals( a:i32,b:i32 )->i32{   a+b  }\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 101, "method": "textDocument/formatting",
        "params": {
            "textDocument": { "uri": &uri },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 101);

    let edits = resp["result"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0]["newText"], "pub fn add_vals(a: i32, b: i32) -> i32 {\n    a + b\n}\n");
}

#[test]
fn test_formatting_already_formatted_code_idempotent() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_fmt_p1_idemp_{}.rs", FMT_P1_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let x = 1;\n    println!(\"{x}\");\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 102, "method": "textDocument/formatting",
        "params": {
            "textDocument": { "uri": &uri },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 102);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_formatting_trailing_whitespace_stripped() {
    if !is_rustfmt_installed() {
        return;
    }
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_fmt_p1_trail_{}.rs", FMT_P1_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {   \n    let x = 10;   \n    let y = 20;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 103, "method": "textDocument/formatting",
        "params": {
            "textDocument": { "uri": &uri },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 103);

    let edits = resp["result"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
    let new_text = edits[0]["newText"].as_str().unwrap();
    assert_eq!(new_text, "fn main() {\n    let x = 10;\n    let y = 20;\n}\n");
}

#[test]
fn test_formatting_consecutive_blank_lines_collapsed() {
    if !is_rustfmt_installed() {
        return;
    }
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_fmt_p1_blanks_{}.rs", FMT_P1_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn first() {}\n\n\n\n\nfn second() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 104, "method": "textDocument/formatting",
        "params": {
            "textDocument": { "uri": &uri },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 104);

    let edits = resp["result"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
    let new_text = edits[0]["newText"].as_str().unwrap();
    assert_eq!(new_text, "fn first() {}\n\nfn second() {}\n");
}

#[test]
fn test_formatting_struct_field_alignment() {
    if !is_rustfmt_installed() {
        return;
    }
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_fmt_p1_struct_{}.rs", FMT_P1_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "pub struct Config{pub host:String,pub port:u16}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 105, "method": "textDocument/formatting",
        "params": {
            "textDocument": { "uri": &uri },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 105);

    let edits = resp["result"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
    let new_text = edits[0]["newText"].as_str().unwrap();
    assert_eq!(new_text, "pub struct Config {\n    pub host: String,\n    pub port: u16,\n}\n");
}

#[test]
fn test_formatting_enum_variant_indentation() {
    if !is_rustfmt_installed() {
        return;
    }
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_fmt_p1_enum_{}.rs", FMT_P1_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "pub enum Action{Start,Stop{force:bool},Pause,}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 106, "method": "textDocument/formatting",
        "params": {
            "textDocument": { "uri": &uri },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 106);

    let edits = resp["result"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
    let new_text = edits[0]["newText"].as_str().unwrap();
    assert_eq!(new_text, "pub enum Action {\n    Start,\n    Stop { force: bool },\n    Pause,\n}\n");
}

#[test]
fn test_formatting_match_arms_indentation() {
    if !is_rustfmt_installed() {
        return;
    }
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_fmt_p1_match_{}.rs", FMT_P1_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn eval(x: i32) -> bool {\nmatch x{1=>true,2=>{true},_=>false}\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 107, "method": "textDocument/formatting",
        "params": {
            "textDocument": { "uri": &uri },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 107);

    let edits = resp["result"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
    let new_text = edits[0]["newText"].as_str().unwrap();
    assert_eq!(new_text, "fn eval(x: i32) -> bool {\n    match x {\n        1 => true,\n        2 => true,\n        _ => false,\n    }\n}\n");
}
