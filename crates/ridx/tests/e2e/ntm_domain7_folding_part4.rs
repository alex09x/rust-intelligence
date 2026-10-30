/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/foldingRange (Domain 7, Part 4).
//! Tests closures, unsafe blocks, async blocks, const blocks, macro invocations, and multi-open lines.

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

static FOLD_P4_COUNTER: AtomicU64 = AtomicU64::new(1);

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
fn test_folding_range_closure_bodies() {
    let id = FOLD_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_fold_p4_clos_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("lib.rs");
    std::fs::write(&file, "fn test_closure() {\n    let c = |x: i32| {\n        x + 1\n    };\n}\n").unwrap();

    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 207, "method": "textDocument/foldingRange",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 207);
    let ranges = resp["result"].as_array().unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0]["startLine"], 1);
    assert_eq!(ranges[0]["endLine"], 3);
    assert_eq!(ranges[1]["startLine"], 0);
    assert_eq!(ranges[1]["endLine"], 4);
}

#[test]
fn test_folding_range_unsafe_blocks() {
    let id = FOLD_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_fold_p4_unsafe_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("lib.rs");
    std::fs::write(&file, "fn do_unsafe() {\n    unsafe {\n        std::ptr::null::<i32>();\n    }\n}\n").unwrap();

    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 208, "method": "textDocument/foldingRange",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 208);
    let ranges = resp["result"].as_array().unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0]["startLine"], 1);
    assert_eq!(ranges[0]["endLine"], 3);
    assert_eq!(ranges[1]["startLine"], 0);
    assert_eq!(ranges[1]["endLine"], 4);
}

#[test]
fn test_folding_range_async_blocks() {
    let id = FOLD_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_fold_p4_async_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("lib.rs");
    std::fs::write(&file, "fn spawn() {\n    let fut = async {\n        42\n    };\n}\n").unwrap();

    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 209, "method": "textDocument/foldingRange",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 209);
    let ranges = resp["result"].as_array().unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0]["startLine"], 1);
    assert_eq!(ranges[0]["endLine"], 3);
    assert_eq!(ranges[1]["startLine"], 0);
    assert_eq!(ranges[1]["endLine"], 4);
}

#[test]
fn test_folding_range_const_blocks() {
    let id = FOLD_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_fold_p4_const_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("lib.rs");
    std::fs::write(&file, "fn with_const() {\n    const {\n        10 + 20\n    };\n}\n").unwrap();

    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 210, "method": "textDocument/foldingRange",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 210);
    let ranges = resp["result"].as_array().unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0]["startLine"], 1);
    assert_eq!(ranges[0]["endLine"], 3);
    assert_eq!(ranges[1]["startLine"], 0);
    assert_eq!(ranges[1]["endLine"], 4);
}

#[test]
fn test_folding_range_macro_invocation_with_braces() {
    let id = FOLD_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_fold_p4_macbr_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("lib.rs");
    std::fs::write(&file, "fn with_vec() {\n    vec! {\n        1,\n        2,\n    };\n}\n").unwrap();

    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 211, "method": "textDocument/foldingRange",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 211);
    let ranges = resp["result"].as_array().unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0]["startLine"], 1);
    assert_eq!(ranges[0]["endLine"], 4);
    assert_eq!(ranges[1]["startLine"], 0);
    assert_eq!(ranges[1]["endLine"], 5);
}

#[test]
fn test_folding_range_multiple_opening_braces_same_line() {
    let id = FOLD_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_fold_p4_multopen_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("lib.rs");
    std::fs::write(&file, "fn outer() { if true {\n    println!();\n}\n}\n").unwrap();

    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 212, "method": "textDocument/foldingRange",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 212);
    let ranges = resp["result"].as_array().unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0]["startLine"], 0);
    assert_eq!(ranges[0]["endLine"], 2);
    assert_eq!(ranges[1]["startLine"], 0);
    assert_eq!(ranges[1]["endLine"], 3);
}
