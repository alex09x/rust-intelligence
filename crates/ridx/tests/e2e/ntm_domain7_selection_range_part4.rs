/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/selectionRange (Domain 7, Part 4).
//! Tests module items, traits, impls, closures, const blocks, and infinite loops.

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

static SEL_P4_COUNTER: AtomicU64 = AtomicU64::new(400);

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
fn test_selection_range_cursor_inside_nested_module() {
    let id = SEL_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_sel_p4_mod_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "mod outer {\n    mod inner {\n        fn nested_fn() {\n            let k = 1;\n        }\n    }\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 401, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 3, "character": 16 }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 401);
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);

    let inner = &results[0];
    assert_eq!(inner["range"]["start"]["line"], 2);
    assert_eq!(inner["range"]["start"]["character"], 11);
    assert_eq!(inner["range"]["end"]["line"], 4);
    assert_eq!(inner["range"]["end"]["character"], 9);

    let mod_inner = &inner["parent"];
    assert_eq!(mod_inner["range"]["start"]["line"], 1);
    assert_eq!(mod_inner["range"]["start"]["character"], 14);
    assert_eq!(mod_inner["range"]["end"]["line"], 5);
    assert_eq!(mod_inner["range"]["end"]["character"], 5);

    let mod_outer = &mod_inner["parent"];
    assert_eq!(mod_outer["range"]["start"]["line"], 0);
    assert_eq!(mod_outer["range"]["start"]["character"], 10);
    assert_eq!(mod_outer["range"]["end"]["line"], 6);
    assert_eq!(mod_outer["range"]["end"]["character"], 1);

    let root_scope = &mod_outer["parent"];
    assert_eq!(root_scope["range"]["start"]["line"], 0);
    assert_eq!(root_scope["range"]["start"]["character"], 0);
    assert_eq!(root_scope["range"]["end"]["line"], 7);
    assert_eq!(root_scope["range"]["end"]["character"], 0);
}

#[test]
fn test_selection_range_cursor_inside_trait_default_method() {
    let id = SEL_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_sel_p4_trait_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "trait Greeter {\n    fn hello(&self) {\n        let msg = \"hi\";\n    }\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 402, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 2, "character": 12 }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 402);
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);

    let inner = &results[0];
    assert_eq!(inner["range"]["start"]["line"], 1);
    assert_eq!(inner["range"]["start"]["character"], 7);
    assert_eq!(inner["range"]["end"]["line"], 3);
    assert_eq!(inner["range"]["end"]["character"], 5);

    let trait_scope = &inner["parent"];
    assert_eq!(trait_scope["range"]["start"]["line"], 0);
    assert_eq!(trait_scope["range"]["start"]["character"], 14);
    assert_eq!(trait_scope["range"]["end"]["line"], 4);
    assert_eq!(trait_scope["range"]["end"]["character"], 1);
}

#[test]
fn test_selection_range_cursor_inside_impl_block_method() {
    let id = SEL_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_sel_p4_impl_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "struct Service;\nimpl Service {\n    fn run(&self) {\n        let active = true;\n    }\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 403, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 3, "character": 12 }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 403);
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);

    let inner = &results[0];
    assert_eq!(inner["range"]["start"]["line"], 2);
    assert_eq!(inner["range"]["start"]["character"], 7);
    assert_eq!(inner["range"]["end"]["line"], 4);
    assert_eq!(inner["range"]["end"]["character"], 5);

    let impl_scope = &inner["parent"];
    assert_eq!(impl_scope["range"]["start"]["line"], 1);
    assert_eq!(impl_scope["range"]["start"]["character"], 13);
    assert_eq!(impl_scope["range"]["end"]["line"], 5);
    assert_eq!(impl_scope["range"]["end"]["character"], 1);
}

#[test]
fn test_selection_range_cursor_inside_closure_block() {
    let id = SEL_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_sel_p4_closure_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn with_closure() {\n    let cb = || {\n        compute();\n    };\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 404, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 2, "character": 10 }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 404);
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
fn test_selection_range_cursor_inside_const_item_block() {
    let id = SEL_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_sel_p4_const_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn with_const() {\n    const LIMIT: i32 = {\n        100\n    };\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 405, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 2, "character": 10 }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 405);
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);

    let inner = &results[0];
    assert_eq!(inner["range"]["start"]["line"], 1);
    assert_eq!(inner["range"]["start"]["character"], 23);
    assert_eq!(inner["range"]["end"]["line"], 3);
    assert_eq!(inner["range"]["end"]["character"], 5);

    let fn_scope = &inner["parent"];
    assert_eq!(fn_scope["range"]["start"]["line"], 0);
    assert_eq!(fn_scope["range"]["start"]["character"], 3);
    assert_eq!(fn_scope["range"]["end"]["line"], 4);
    assert_eq!(fn_scope["range"]["end"]["character"], 1);
}

#[test]
fn test_selection_range_cursor_inside_loop_infinite() {
    let id = SEL_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_sel_p4_loop_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn infinite() {\n    loop {\n        tick();\n    }\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 406, "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "positions": [{ "line": 2, "character": 10 }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 406);
    let results = resp["result"].as_array().unwrap();
    assert_eq!(results.len(), 1);

    let inner = &results[0];
    assert_eq!(inner["range"]["start"]["line"], 1);
    assert_eq!(inner["range"]["start"]["character"], 9);
    assert_eq!(inner["range"]["end"]["line"], 3);
    assert_eq!(inner["range"]["end"]["character"], 5);

    let fn_scope = &inner["parent"];
    assert_eq!(fn_scope["range"]["start"]["line"], 0);
    assert_eq!(fn_scope["range"]["start"]["character"], 3);
    assert_eq!(fn_scope["range"]["end"]["line"], 4);
    assert_eq!(fn_scope["range"]["end"]["character"], 1);
}
