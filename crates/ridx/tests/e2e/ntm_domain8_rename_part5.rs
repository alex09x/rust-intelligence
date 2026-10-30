/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/rename (Domain 8, Part 5).
//! Tests protocol adherence, lifecycle states, cancellation, ID preservation,
//! documentChanges version tracking, and isolation.

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

static REN_P5_COUNTER: AtomicU64 = AtomicU64::new(1);

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
fn test_rename_after_shutdown_returns_code_32600() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ren_p5_shut_{}.rs", REN_P5_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let shutdown_msg = json!({
        "jsonrpc": "2.0", "id": 500, "method": "shutdown"
    });
    assert!(dsp.dispatch(&shutdown_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 501, "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 0 },
            "newName": "new_name"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 501);
    assert_eq!(resp["error"]["code"], -32600);
}

#[test]
fn test_rename_cancelled_request_returns_code_32800() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ren_p5_cancel_{}.rs", REN_P5_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let cancel_id = json!(502);
    dsp.cancelled.lock().unwrap().insert(cancel_id.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": cancel_id, "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 0 },
            "newName": "new_name"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 502);
    assert_eq!(resp["error"]["code"], -32800);
}

#[test]
fn test_rename_jsonrpc_protocol_version_is_2_0() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ren_p5_ver_{}.rs", REN_P5_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let my_val = 1;\n    my_val + 2;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 503, "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 9 },
            "newName": "updated_val"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 503);
    assert_eq!(
        resp["result"]["changes"][&uri],
        json!([
            {
                "range": {
                    "start": { "line": 1, "character": 8 },
                    "end": { "line": 1, "character": 14 }
                },
                "newText": "updated_val"
            },
            {
                "range": {
                    "start": { "line": 2, "character": 4 },
                    "end": { "line": 2, "character": 10 }
                },
                "newText": "updated_val"
            }
        ])
    );
}

#[test]
fn test_rename_large_numeric_id_preserved() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ren_p5_largeid_{}.rs", REN_P5_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let my_val = 1;\n    my_val + 2;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let large_id: u64 = 9_000_000_000_000_000_000;
    let req = json!({
        "jsonrpc": "2.0", "id": large_id, "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 9 },
            "newName": "updated_val"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], large_id);
}

#[test]
fn test_rename_string_id_preserved() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ren_p5_strid_{}.rs", REN_P5_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let my_val = 1;\n    my_val + 2;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let str_id = "req-rename-custom-id-505";
    let req = json!({
        "jsonrpc": "2.0", "id": str_id, "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 9 },
            "newName": "updated_val"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], str_id);
}

#[test]
fn test_rename_negative_numeric_id_preserved() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ren_p5_negid_{}.rs", REN_P5_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let my_val = 1;\n    my_val + 2;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let neg_id = -42;
    let req = json!({
        "jsonrpc": "2.0", "id": neg_id, "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 9 },
            "newName": "updated_val"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], neg_id);
}

#[test]
fn test_rename_document_changes_reflects_overlay_version() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ren_p5_vertrack_{}.rs", REN_P5_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 42,
                "text": "fn main() {\n    let my_val = 1;\n    my_val + 2;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 507, "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 9 },
            "newName": "tracked_val"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 507);

    let doc_changes = resp["result"]["documentChanges"].as_array().unwrap();
    assert_eq!(doc_changes.len(), 1);
    assert_eq!(doc_changes[0]["textDocument"]["uri"], uri);
    assert_eq!(doc_changes[0]["textDocument"]["version"], 42);
}

#[test]
fn test_rename_consecutive_requests_isolation() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ren_p5_consec_{}.rs", REN_P5_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let a = 1;\n    let b = 2;\n    a + b;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req1 = json!({
        "jsonrpc": "2.0", "id": 508, "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 9 },
            "newName": "first_var"
        }
    });
    assert!(dsp.dispatch(&req1));
    let resp1 = dsp.parse_response();
    assert_eq!(resp1["id"], 508);
    let edits1 = resp1["result"]["changes"][&uri].as_array().unwrap();
    assert_eq!(edits1.len(), 2);
    assert_eq!(edits1[0]["newText"], "first_var");

    let req2 = json!({
        "jsonrpc": "2.0", "id": 509, "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 9 },
            "newName": "second_var"
        }
    });
    assert!(dsp.dispatch(&req2));
    let resp2 = dsp.parse_response();
    assert_eq!(resp2["id"], 509);
    let edits2 = resp2["result"]["changes"][&uri].as_array().unwrap();
    assert_eq!(edits2.len(), 2);
    assert_eq!(edits2[0]["newText"], "second_var");
}

#[test]
fn test_rename_to_identical_name_succeeds_with_edits() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_ren_p5_ident_{}.rs", REN_P5_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &uri, "languageId": "rust", "version": 1,
                "text": "fn main() {\n    let unchanged = 1;\n    unchanged + 1;\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 510, "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 9 },
            "newName": "unchanged"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 510);

    let edits = resp["result"]["changes"][&uri].as_array().unwrap();
    assert_eq!(edits.len(), 2);
    assert_eq!(edits[0]["newText"], "unchanged");
    assert_eq!(edits[1]["newText"], "unchanged");
}
