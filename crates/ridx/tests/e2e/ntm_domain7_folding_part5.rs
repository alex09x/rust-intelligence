/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/foldingRange (Domain 7, Part 5).
//! Tests multi-close on same line, enum struct variants, impl blocks, modules, traits, and line gaps.

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

static FOLD_P5_COUNTER: AtomicU64 = AtomicU64::new(1);

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
fn test_folding_range_multiple_closing_braces_same_line() {
    let id = FOLD_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_fold_p5_multclose_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("lib.rs");
    std::fs::write(&file, "fn outer() {\n    if true {\n        1;\n    } }\n").unwrap();

    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 213, "method": "textDocument/foldingRange",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 213);
    let ranges = resp["result"].as_array().unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0]["startLine"], 1);
    assert_eq!(ranges[0]["endLine"], 3);
    assert_eq!(ranges[1]["startLine"], 0);
    assert_eq!(ranges[1]["endLine"], 3);
}

#[test]
fn test_folding_range_enum_with_struct_variants() {
    let id = FOLD_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_fold_p5_enumvar_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("lib.rs");
    std::fs::write(&file, "enum Message {\n    Move {\n        x: i32,\n        y: i32,\n    },\n    Quit,\n}\n").unwrap();

    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 214, "method": "textDocument/foldingRange",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 214);
    let ranges = resp["result"].as_array().unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0]["startLine"], 1);
    assert_eq!(ranges[0]["endLine"], 4);
    assert_eq!(ranges[1]["startLine"], 0);
    assert_eq!(ranges[1]["endLine"], 6);
}

#[test]
fn test_folding_range_impl_with_multiple_methods() {
    let id = FOLD_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_fold_p5_implm_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("lib.rs");
    std::fs::write(&file, "impl MyType {\n    fn a(&self) {\n        1;\n    }\n    fn b(&self) {\n        2;\n    }\n}\n").unwrap();

    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 215, "method": "textDocument/foldingRange",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 215);
    let ranges = resp["result"].as_array().unwrap();
    assert_eq!(ranges.len(), 3);
    assert_eq!(ranges[0]["startLine"], 1);
    assert_eq!(ranges[0]["endLine"], 3);
    assert_eq!(ranges[1]["startLine"], 4);
    assert_eq!(ranges[1]["endLine"], 6);
    assert_eq!(ranges[2]["startLine"], 0);
    assert_eq!(ranges[2]["endLine"], 7);
}

#[test]
fn test_folding_range_module_nested_in_module() {
    let id = FOLD_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_fold_p5_nestmod_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("lib.rs");
    std::fs::write(&file, "mod outer {\n    mod inner {\n        pub fn deep() {}\n    }\n}\n").unwrap();

    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 216, "method": "textDocument/foldingRange",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 216);
    let ranges = resp["result"].as_array().unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0]["startLine"], 1);
    assert_eq!(ranges[0]["endLine"], 3);
    assert_eq!(ranges[1]["startLine"], 0);
    assert_eq!(ranges[1]["endLine"], 4);
}

#[test]
fn test_folding_range_trait_with_default_methods() {
    let id = FOLD_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_fold_p5_traitdef_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("lib.rs");
    std::fs::write(&file, "trait Greet {\n    fn greet(&self) {\n        println!();\n    }\n}\n").unwrap();

    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 217, "method": "textDocument/foldingRange",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 217);
    let ranges = resp["result"].as_array().unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0]["startLine"], 1);
    assert_eq!(ranges[0]["endLine"], 3);
    assert_eq!(ranges[1]["startLine"], 0);
    assert_eq!(ranges[1]["endLine"], 4);
}

#[test]
fn test_folding_range_empty_lines_between_functions() {
    let id = FOLD_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_fold_p5_gap_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("lib.rs");
    std::fs::write(&file, "fn first() {\n    1;\n}\n\n\nfn second() {\n    2;\n}\n").unwrap();

    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 218, "method": "textDocument/foldingRange",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 218);
    let ranges = resp["result"].as_array().unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0]["startLine"], 0);
    assert_eq!(ranges[0]["endLine"], 2);
    assert_eq!(ranges[1]["startLine"], 5);
    assert_eq!(ranges[1]["endLine"], 7);
}
