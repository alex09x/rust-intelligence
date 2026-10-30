/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspace/executeCommand (Domain 8, Part 2).
//! Tests ridx.applyEdits planning, arguments validation, and workspace/applyEdit requests.

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

static EXEC_P2_COUNTER: AtomicU64 = AtomicU64::new(1);

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
fn test_execute_command_apply_edits_empty_arguments_fails() {
    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 201, "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.applyEdits",
            "arguments": []
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 201);
    assert_eq!(resp["result"]["applied"], false);
    assert_eq!(resp["result"]["failure"], "no_arguments");
}

#[test]
fn test_execute_command_apply_edits_invalid_workspace_edit_structure() {
    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 202, "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [
                { "invalid_key": "not a workspace edit" }
            ]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 202);
    assert_eq!(resp["result"]["applied"], false);
}

#[test]
fn test_execute_command_apply_edits_single_file_overlay_plan() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_exec_p2_ov_{}.rs", EXEC_P2_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": { "uri": &uri, "languageId": "rust", "version": 1, "text": "let x = 1;" }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 203, "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [
                {
                    "changes": {
                        &uri: [
                            {
                                "range": { "start": { "line": 0, "character": 4 }, "end": { "line": 0, "character": 5 } },
                                "newText": "y"
                            }
                        ]
                    }
                }
            ]
        }
    });
    assert!(dsp.dispatch(&req));
    let out = dsp.parse_response();
    assert_eq!(out["method"], "workspace/applyEdit");
}

#[test]
fn test_execute_command_apply_edits_single_file_disk_plan() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_exec_p2_disk_{}.rs", EXEC_P2_COUNTER.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&file, "const VAL: i32 = 10;").unwrap();
    let uri = to_uri(&file);

    let req = json!({
        "jsonrpc": "2.0", "id": 204, "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [
                {
                    "changes": {
                        &uri: [
                            {
                                "range": { "start": { "line": 0, "character": 6 }, "end": { "line": 0, "character": 9 } },
                                "newText": "NEW_VAL"
                            }
                        ]
                    }
                }
            ]
        }
    });
    assert!(dsp.dispatch(&req));
    let out = dsp.parse_response();
    assert_eq!(out["method"], "workspace/applyEdit");

    let _ = std::fs::remove_file(&file);
}

#[test]
fn test_execute_command_apply_edits_multi_file_mixed_plan() {
    let mut dsp = TestDispatcher::new();
    let file1 = std::env::temp_dir().join(format!("ridx_exec_p2_m1_{}.rs", EXEC_P2_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let file2 = std::env::temp_dir().join(format!("ridx_exec_p2_m2_{}.rs", EXEC_P2_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri1 = to_uri(&file1);
    let uri2 = to_uri(&file2);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": { "uri": &uri1, "languageId": "rust", "version": 1, "text": "fn one() {}" }
        }
    });
    assert!(dsp.dispatch(&open_msg));
    std::fs::write(&file2, "fn two() {}").unwrap();

    let req = json!({
        "jsonrpc": "2.0", "id": 205, "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [
                {
                    "changes": {
                        &uri1: [{ "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 6 } }, "newText": "first" }],
                        &uri2: [{ "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 6 } }, "newText": "second" }]
                    }
                }
            ]
        }
    });
    assert!(dsp.dispatch(&req));
    let out = dsp.parse_response();
    assert_eq!(out["method"], "workspace/applyEdit");

    let _ = std::fs::remove_file(&file2);
}

#[test]
fn test_execute_command_apply_edits_version_match_succeeds() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_exec_p2_verok_{}.rs", EXEC_P2_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": { "uri": &uri, "languageId": "rust", "version": 5, "text": "fn ver() {}" }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 206, "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [
                {
                    "documentChanges": [
                        {
                            "textDocument": { "uri": &uri, "version": 5 },
                            "edits": [
                                { "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 6 } }, "newText": "valid" }
                            ]
                        }
                    ]
                }
            ]
        }
    });
    assert!(dsp.dispatch(&req));
    let out = dsp.parse_response();
    assert_eq!(out["method"], "workspace/applyEdit");
}

#[test]
fn test_execute_command_apply_edits_stale_version_rejected() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_exec_p2_stale_{}.rs", EXEC_P2_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": { "uri": &uri, "languageId": "rust", "version": 3, "text": "fn ver() {}" }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 207, "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [
                {
                    "documentChanges": [
                        {
                            "textDocument": { "uri": &uri, "version": 2 },
                            "edits": [
                                { "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 6 } }, "newText": "stale" }
                            ]
                        }
                    ]
                }
            ]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 207);
    assert_eq!(resp["result"]["applied"], false);
    assert_eq!(resp["result"]["failure"], "version_mismatch");
}

#[test]
fn test_execute_command_apply_edits_multiple_edits_same_file_accumulated() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_exec_p2_multiedit_{}.rs", EXEC_P2_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": { "uri": &uri, "languageId": "rust", "version": 1, "text": "fn abc() { 123 }" }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 208, "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [
                {
                    "changes": {
                        &uri: [
                            { "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 6 } }, "newText": "xyz" },
                            { "range": { "start": { "line": 0, "character": 11 }, "end": { "line": 0, "character": 14 } }, "newText": "456" }
                        ]
                    }
                }
            ]
        }
    });
    assert!(dsp.dispatch(&req));
    let out = dsp.parse_response();
    assert_eq!(out["method"], "workspace/applyEdit");
}

#[test]
fn test_execute_command_apply_edits_generates_apply_edit_request() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_exec_p2_req_{}.rs", EXEC_P2_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": { "uri": &uri, "languageId": "rust", "version": 1, "text": "let a = 1;" }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 209, "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [
                {
                    "changes": {
                        &uri: [
                            { "range": { "start": { "line": 0, "character": 4 }, "end": { "line": 0, "character": 5 } }, "newText": "b" }
                        ]
                    }
                }
            ]
        }
    });
    assert!(dsp.dispatch(&req));
    let out = dsp.parse_response();
    assert_eq!(out["jsonrpc"], "2.0");
    assert_eq!(out["method"], "workspace/applyEdit");
    assert_eq!(out["params"]["label"], "Apply refactoring edits");
}
