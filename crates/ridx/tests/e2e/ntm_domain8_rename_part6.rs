/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/rename (Domain 8, Part 6).
//! Tests local binding shadowing isolation, Greek multibyte characters, and CRLF line endings.

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

static REN_P6_COUNTER: AtomicU64 = AtomicU64::new(1);

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
fn test_rename_nested_block_shadowed_inner_binding_leaves_outer_untouched() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ren_p6_shadin_{}.rs", REN_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let shadowed = 1;\n    {\n        let shadowed = 2;\n        shadowed + 10;\n    }\n    shadowed + 20;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 206, "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 3, "character": 13 },
            "newName": "inner_val"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 206);

    let edits = resp["result"]["changes"][&uri].as_array().unwrap();
    assert_eq!(edits.len(), 2);
    assert_eq!(edits[0]["range"]["start"]["line"], 3);
    assert_eq!(edits[0]["newText"], "inner_val");
    assert_eq!(edits[1]["range"]["start"]["line"], 4);
    assert_eq!(edits[1]["newText"], "inner_val");
}

#[test]
fn test_rename_nested_block_outer_binding_leaves_inner_shadowed_untouched() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ren_p6_shadout_{}.rs", REN_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let outer = 1;\n    {\n        let outer = 2;\n        outer + 10;\n    }\n    outer + 20;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 207, "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 9 },
            "newName": "base"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 207);

    let edits = resp["result"]["changes"][&uri].as_array().unwrap();
    assert_eq!(edits.len(), 2);
    assert_eq!(edits[0]["range"]["start"]["line"], 1);
    assert_eq!(edits[0]["newText"], "base");
    assert_eq!(edits[1]["range"]["start"]["line"], 6);
    assert_eq!(edits[1]["newText"], "base");
}

#[test]
fn test_rename_greek_multibyte_prefix_columns() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ren_p6_greek_{}.rs", REN_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    // Greek comment: Γειάσα κόσμε\n    let alpha = 42;\n    alpha + 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 208, "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 9 },
            "newName": "beta"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 208);

    let edits = resp["result"]["changes"][&uri].as_array().unwrap();
    assert_eq!(edits.len(), 2);
    assert_eq!(edits[0]["range"]["start"]["line"], 2);
    assert_eq!(edits[0]["newText"], "beta");
    assert_eq!(edits[1]["range"]["start"]["line"], 3);
    assert_eq!(edits[1]["newText"], "beta");
}

#[test]
fn test_rename_crlf_line_endings_preserved() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ren_p6_crlf_{}.rs", REN_P6_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\r\n    let crlf_var = 99;\r\n    crlf_var + 1;\r\n}\r\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 209, "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 9 },
            "newName": "renamed_crlf"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 209);

    let edits = resp["result"]["changes"][&uri].as_array().unwrap();
    assert_eq!(edits.len(), 2);
    assert_eq!(edits[0]["range"]["start"]["line"], 1);
    assert_eq!(edits[0]["newText"], "renamed_crlf");
    assert_eq!(edits[1]["range"]["start"]["line"], 2);
    assert_eq!(edits[1]["newText"], "renamed_crlf");
}
