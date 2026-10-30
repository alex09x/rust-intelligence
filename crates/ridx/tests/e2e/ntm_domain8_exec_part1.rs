/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspace/executeCommand (Domain 8, Part 1).
//! Tests ridx.resolveWorkspace and ridx.showReferences parameter parsing and responses.

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

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
fn test_execute_command_resolve_workspace_with_root_uri() {
    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 101, "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.resolveWorkspace",
            "rootUri": "file:///path/to/my_workspace"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 101);
    assert_eq!(resp["result"]["resolved"], true);
    assert_eq!(resp["result"]["workspaceRoot"], "file:///path/to/my_workspace");
    assert_eq!(resp["result"]["status"], "ready");
}

#[test]
fn test_execute_command_resolve_workspace_with_root_path() {
    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 102, "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.resolveWorkspace",
            "rootPath": "/workspace/project"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 102);
    assert_eq!(resp["result"]["resolved"], true);
    assert_eq!(resp["result"]["workspaceRoot"], "/workspace/project");
    assert_eq!(resp["result"]["status"], "ready");
}

#[test]
fn test_execute_command_resolve_workspace_empty_params() {
    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 103, "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.resolveWorkspace"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 103);
    assert_eq!(resp["result"]["resolved"], true);
    assert_eq!(resp["result"]["workspaceRoot"], "");
    assert_eq!(resp["result"]["status"], "ready");
}

#[test]
fn test_execute_command_resolve_workspace_custom_uri_scheme() {
    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 104, "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.resolveWorkspace",
            "rootUri": "vscode-vfs://remote/project"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 104);
    assert_eq!(resp["result"]["resolved"], true);
    assert_eq!(resp["result"]["workspaceRoot"], "vscode-vfs://remote/project");
    assert_eq!(resp["result"]["status"], "ready");
}

#[test]
fn test_execute_command_show_references_with_position_and_locations() {
    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 105, "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.showReferences",
            "arguments": [
                "file:///workspace/src/main.rs",
                { "line": 10, "character": 4 },
                [
                    {
                        "uri": "file:///workspace/src/lib.rs",
                        "range": {
                            "start": { "line": 5, "character": 0 },
                            "end": { "line": 5, "character": 10 }
                        }
                    }
                ]
            ]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 105);
    assert_eq!(resp["result"]["success"], true);
    assert_eq!(resp["result"]["uri"], "file:///workspace/src/main.rs");
    assert_eq!(resp["result"]["position"]["line"], 10);
    assert_eq!(resp["result"]["position"]["character"], 4);
    assert_eq!(resp["result"]["count"], 1);
    let locs = resp["result"]["locations"].as_array().unwrap();
    assert_eq!(locs.len(), 1);
    assert_eq!(locs[0]["uri"], "file:///workspace/src/lib.rs");
}

#[test]
fn test_execute_command_show_references_empty_locations() {
    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 106, "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.showReferences",
            "arguments": [
                "file:///workspace/src/main.rs",
                { "line": 0, "character": 0 },
                []
            ]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 106);
    assert_eq!(resp["result"]["success"], true);
    assert_eq!(resp["result"]["count"], 0);
    assert_eq!(resp["result"]["locations"], json!([]));
}

#[test]
fn test_execute_command_show_references_array_arguments_position() {
    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 107, "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.showReferences",
            "arguments": [
                "file:///workspace/tests.rs",
                { "line": 42, "character": 18 }
            ]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 107);
    assert_eq!(resp["result"]["success"], true);
    assert_eq!(resp["result"]["uri"], "file:///workspace/tests.rs");
    assert_eq!(resp["result"]["position"]["line"], 42);
    assert_eq!(resp["result"]["position"]["character"], 18);
    assert_eq!(resp["result"]["count"], 0);
}

#[test]
fn test_execute_command_show_references_multiple_locations_count() {
    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 108, "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.showReferences",
            "arguments": [
                "file:///workspace/src/foo.rs",
                { "line": 1, "character": 2 },
                [
                    { "uri": "file:///workspace/src/a.rs", "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 5 } } },
                    { "uri": "file:///workspace/src/b.rs", "range": { "start": { "line": 1, "character": 0 }, "end": { "line": 1, "character": 5 } } },
                    { "uri": "file:///workspace/src/c.rs", "range": { "start": { "line": 2, "character": 0 }, "end": { "line": 2, "character": 5 } } }
                ]
            ]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 108);
    assert_eq!(resp["result"]["success"], true);
    assert_eq!(resp["result"]["count"], 3);
}

#[test]
fn test_execute_command_show_references_named_params_fallback() {
    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 109, "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.showReferences",
            "uri": "file:///workspace/named.rs",
            "position": { "line": 3, "character": 7 },
            "locations": [
                { "uri": "file:///workspace/target.rs", "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } } }
            ]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 109);
    assert_eq!(resp["result"]["success"], true);
    assert_eq!(resp["result"]["uri"], "file:///workspace/named.rs");
    assert_eq!(resp["result"]["position"]["line"], 3);
    assert_eq!(resp["result"]["position"]["character"], 7);
    assert_eq!(resp["result"]["count"], 1);
}
