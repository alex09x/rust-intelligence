/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/selectionRange (Domain 7, Part 3).
//! Tests control flow and block constructs: match, for, while, if/else, let-block, unsafe.

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::path_to_uri;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use serde_json::json;
use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static SEL_P3_COUNTER: AtomicU64 = AtomicU64::new(300);

fn to_uri(p: &Path) -> String {
    path_to_uri(p.to_str().unwrap())
}

struct TestDispatcher {
    lifecycle: LifecycleManager,
    multi_root: MultiRootWorkspace,
    overlay_mgr: OverlayManager,
    cancelled: Arc<Mutex<HashSet<serde_json::Value>>>,
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

    fn dispatch(&mut self, msg: &serde_json::Value) -> bool {
        dispatch_lsp_message(
            msg.clone(), &mut self.writer, &mut self.lifecycle,
            &mut self.multi_root, &mut self.overlay_mgr, &self.cancelled, None,
        ).unwrap()
    }

    fn parse_response(&mut self) -> serde_json::Value {
        let text = String::from_utf8(self.writer.clone()).unwrap();
        self.writer.clear();
        let body = text.split("\r\n\r\n").nth(1).unwrap();
        serde_json::from_str(body).unwrap()
    }
}

#[test]
fn test_selection_range_cursor_inside_match_arm_block() {
    let id = SEL_P3_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_sel_p3_match_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn eval(x: i32) {\n    match x {\n        1 => {\n            do_one();\n        }\n        _ => (),\n    }\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 301, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 3, "character": 14 }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 301);
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);

    let inner = &results[0];
    assert_eq!(inner["range"]["start"]["line"], 2);
    assert_eq!(inner["range"]["start"]["character"], 13);
    assert_eq!(inner["range"]["end"]["line"], 4);
    assert_eq!(inner["range"]["end"]["character"], 9);

    let match_block = &inner["parent"];
    assert_eq!(match_block["range"]["start"]["line"], 1);
    assert_eq!(match_block["range"]["start"]["character"], 12);
    assert_eq!(match_block["range"]["end"]["line"], 6);
    assert_eq!(match_block["range"]["end"]["character"], 5);

    let fn_scope = &match_block["parent"];
    assert_eq!(fn_scope["range"]["start"]["line"], 0);
    assert_eq!(fn_scope["range"]["start"]["character"], 3);
    assert_eq!(fn_scope["range"]["end"]["line"], 7);
    assert_eq!(fn_scope["range"]["end"]["character"], 1);

    let root_scope = &fn_scope["parent"];
    assert_eq!(root_scope["range"]["start"]["line"], 0);
    assert_eq!(root_scope["range"]["start"]["character"], 0);
    assert_eq!(root_scope["range"]["end"]["line"], 8);
    assert_eq!(root_scope["range"]["end"]["character"], 0);
    assert_eq!(root_scope["parent"], serde_json::Value::Null);
}

#[test]
fn test_selection_range_cursor_inside_for_loop_block() {
    let id = SEL_P3_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_sel_p3_for_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn loop_test() {\n    for i in 0..10 {\n        let square = i * i;\n    }\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 302, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 2, "character": 12 }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 302);
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);

    let inner = &results[0];
    assert_eq!(inner["range"]["start"]["line"], 1);
    assert_eq!(inner["range"]["start"]["character"], 19);
    assert_eq!(inner["range"]["end"]["line"], 3);
    assert_eq!(inner["range"]["end"]["character"], 5);

    let fn_scope = &inner["parent"];
    assert_eq!(fn_scope["range"]["start"]["line"], 0);
    assert_eq!(fn_scope["range"]["start"]["character"], 3);
    assert_eq!(fn_scope["range"]["end"]["line"], 4);
    assert_eq!(fn_scope["range"]["end"]["character"], 1);

    let root_scope = &fn_scope["parent"];
    assert_eq!(root_scope["range"]["start"]["line"], 0);
    assert_eq!(root_scope["range"]["start"]["character"], 0);
    assert_eq!(root_scope["range"]["end"]["line"], 5);
    assert_eq!(root_scope["range"]["end"]["character"], 0);
    assert_eq!(root_scope["parent"], serde_json::Value::Null);
}

#[test]
fn test_selection_range_cursor_inside_while_block() {
    let id = SEL_P3_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_sel_p3_while_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn while_test() {\n    while true {\n        step();\n    }\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 303, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 2, "character": 10 }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 303);
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);

    let inner = &results[0];
    assert_eq!(inner["range"]["start"]["line"], 1);
    assert_eq!(inner["range"]["start"]["character"], 15);
    assert_eq!(inner["range"]["end"]["line"], 3);
    assert_eq!(inner["range"]["end"]["character"], 5);

    let fn_scope = &inner["parent"];
    assert_eq!(fn_scope["range"]["start"]["line"], 0);
    assert_eq!(fn_scope["range"]["start"]["character"], 3);
    assert_eq!(fn_scope["range"]["end"]["line"], 4);
    assert_eq!(fn_scope["range"]["end"]["character"], 1);
}

#[test]
fn test_selection_range_cursor_inside_if_else_blocks() {
    let id = SEL_P3_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_sel_p3_ifelse_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn branch(cond: bool) {\n    if cond {\n        do_then();\n    } else {\n        do_else();\n    }\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 304, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 4, "character": 10 }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 304);
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);

    let inner = &results[0];
    assert_eq!(inner["range"]["start"]["line"], 3);
    assert_eq!(inner["range"]["start"]["character"], 11);
    assert_eq!(inner["range"]["end"]["line"], 5);
    assert_eq!(inner["range"]["end"]["character"], 5);

    let fn_scope = &inner["parent"];
    assert_eq!(fn_scope["range"]["start"]["line"], 0);
    assert_eq!(fn_scope["range"]["start"]["character"], 3);
    assert_eq!(fn_scope["range"]["end"]["line"], 6);
    assert_eq!(fn_scope["range"]["end"]["character"], 1);
}

#[test]
fn test_selection_range_cursor_inside_block_expression() {
    let id = SEL_P3_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_sel_p3_blockexpr_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn block_run() {\n    let block = {\n        perform();\n    };\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 305, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 2, "character": 10 }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 305);
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);

    let inner = &results[0];
    assert_eq!(inner["range"]["start"]["line"], 1);
    assert_eq!(inner["range"]["start"]["character"], 16);
    assert_eq!(inner["range"]["end"]["line"], 3);
    assert_eq!(inner["range"]["end"]["character"], 5);

    let fn_scope = &inner["parent"];
    assert_eq!(fn_scope["range"]["start"]["line"], 0);
    assert_eq!(fn_scope["range"]["start"]["character"], 3);
    assert_eq!(fn_scope["range"]["end"]["line"], 4);
    assert_eq!(fn_scope["range"]["end"]["character"], 1);
}

#[test]
fn test_selection_range_cursor_inside_unsafe_block() {
    let id = SEL_P3_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_sel_p3_unsafe_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn unsafe_call() {\n    unsafe {\n        ptr_op();\n    }\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 306, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 2, "character": 10 }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 306);
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);

    let inner = &results[0];
    assert_eq!(inner["range"]["start"]["line"], 1);
    assert_eq!(inner["range"]["start"]["character"], 11);
    assert_eq!(inner["range"]["end"]["line"], 3);
    assert_eq!(inner["range"]["end"]["character"], 5);

    let fn_scope = &inner["parent"];
    assert_eq!(fn_scope["range"]["start"]["line"], 0);
    assert_eq!(fn_scope["range"]["start"]["character"], 3);
    assert_eq!(fn_scope["range"]["end"]["line"], 4);
    assert_eq!(fn_scope["range"]["end"]["character"], 1);
}
