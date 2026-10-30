/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/prepareRename (Domain 8, Part 5).
//! Tests local scopes, shadowing, mutable bindings, typed let, function parameters, and didChange range shifts.

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

static PREP_P5_COUNTER: AtomicU64 = AtomicU64::new(1);

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
fn test_prepare_rename_nested_block_shadowed_inner_binding() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_prep_p5_shadow_{}.rs", PREP_P5_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn test() {\n    let x = 1;\n    {\n        let x = 2;\n        x + 1;\n    }\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 501, "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 3, "character": 12 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 501);
    assert_eq!(resp["result"]["placeholder"], "x");
    assert_eq!(resp["result"]["range"]["start"]["line"], 3);
    assert_eq!(resp["result"]["range"]["start"]["character"], 12);
    assert_eq!(resp["result"]["range"]["end"]["line"], 3);
    assert_eq!(resp["result"]["range"]["end"]["character"], 13);
}

#[test]
fn test_prepare_rename_nested_block_outer_binding_unaffected() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_prep_p5_outer_{}.rs", PREP_P5_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn test() {\n    let x = 1;\n    {\n        let x = 2;\n    }\n    x + 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 502, "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 1, "character": 8 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 502);
    assert_eq!(resp["result"]["placeholder"], "x");
    assert_eq!(resp["result"]["range"]["start"]["line"], 1);
    assert_eq!(resp["result"]["range"]["start"]["character"], 8);
    assert_eq!(resp["result"]["range"]["end"]["line"], 1);
    assert_eq!(resp["result"]["range"]["end"]["character"], 9);
}

#[test]
fn test_prepare_rename_mut_let_binding_exact_range() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_prep_p5_mut_{}.rs", PREP_P5_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let mut counter = 10;\n    counter += 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 503, "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 1, "character": 13 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 503);
    assert_eq!(resp["result"]["placeholder"], "counter");
    assert_eq!(resp["result"]["range"]["start"]["line"], 1);
    assert_eq!(resp["result"]["range"]["start"]["character"], 12);
    assert_eq!(resp["result"]["range"]["end"]["line"], 1);
    assert_eq!(resp["result"]["range"]["end"]["character"], 19);
}

#[test]
fn test_prepare_rename_explicit_type_annotation_let_binding() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_prep_p5_typed_{}.rs", PREP_P5_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let count: usize = 100;\n    count + 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 504, "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 1, "character": 9 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 504);
    assert_eq!(resp["result"]["placeholder"], "count");
    assert_eq!(resp["result"]["range"]["start"]["line"], 1);
    assert_eq!(resp["result"]["range"]["start"]["character"], 8);
    assert_eq!(resp["result"]["range"]["end"]["line"], 1);
    assert_eq!(resp["result"]["range"]["end"]["character"], 13);
}

#[test]
fn test_prepare_rename_function_second_parameter() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_prep_p5_param2_{}.rs", PREP_P5_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn process(first: i32, second_arg: &str) -> i32 {\n    second_arg.len() as i32\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 505, "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 25 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 505);
    assert_eq!(resp["result"]["placeholder"], "second_arg");
    assert_eq!(resp["result"]["range"]["start"]["line"], 0);
    assert_eq!(resp["result"]["range"]["start"]["character"], 23);
    assert_eq!(resp["result"]["range"]["end"]["line"], 0);
    assert_eq!(resp["result"]["range"]["end"]["character"], 33);
}

#[test]
fn test_prepare_rename_did_change_overlay_moves_binding_range() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_prep_p5_shift_{}.rs", PREP_P5_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let shifting = 1;\n    shifting + 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let change_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": to_uri(&file), "version": 2 },
            "contentChanges": [{
                "text": "// Header 1\n// Header 2\nfn main() {\n    let shifting = 1;\n    shifting + 1;\n}\n"
            }]
        }
    });
    assert!(dsp.dispatch(&change_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 506, "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 3, "character": 9 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 506);
    assert_eq!(resp["result"]["placeholder"], "shifting");
    assert_eq!(resp["result"]["range"]["start"]["line"], 3);
    assert_eq!(resp["result"]["range"]["start"]["character"], 8);
    assert_eq!(resp["result"]["range"]["end"]["line"], 3);
    assert_eq!(resp["result"]["range"]["end"]["character"], 16);
}
