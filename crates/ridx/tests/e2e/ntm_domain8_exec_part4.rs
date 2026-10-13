/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspace/executeCommand (Domain 8, Part 4).
//! Tests JSON-RPC IDs, protocol version, two-way apply edit accept/reject handshakes, and idempotence.

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

static EXEC_P4_COUNTER: AtomicU64 = AtomicU64::new(1);

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
fn test_execute_command_string_id_preserved() {
    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": "cmd-req-abc-123", "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.resolveWorkspace",
            "rootUri": "file:///test"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], "cmd-req-abc-123");
}

#[test]
fn test_execute_command_large_numeric_id_preserved() {
    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 8_888_888_888u64, "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.resolveWorkspace",
            "rootUri": "file:///large"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 8_888_888_888u64);
}

#[test]
fn test_execute_command_negative_numeric_id_preserved() {
    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": -99, "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.resolveWorkspace",
            "rootUri": "file:///neg"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], -99);
}

#[test]
fn test_execute_command_jsonrpc_protocol_version_is_2_0() {
    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 401, "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.resolveWorkspace"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
}

#[test]
fn test_execute_command_apply_edits_client_accept_commits_edits() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_exec_p4_acc_{}.rs", EXEC_P4_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": { "uri": &uri, "languageId": "rust", "version": 1, "text": "let x = 1;" }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let cmd = json!({
        "jsonrpc": "2.0", "id": 402, "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [
                {
                    "changes": {
                        &uri: [
                            { "range": { "start": { "line": 0, "character": 4 }, "end": { "line": 0, "character": 5 } }, "newText": "renamed_var" }
                        ]
                    }
                }
            ]
        }
    });
    assert!(dsp.dispatch(&cmd));
    let apply_req = dsp.parse_response();
    assert_eq!(apply_req["method"], "workspace/applyEdit");
    let apply_id = apply_req["id"].clone();

    let client_ack = json!({
        "jsonrpc": "2.0",
        "id": apply_id,
        "result": { "applied": true }
    });
    assert!(dsp.dispatch(&client_ack));
    let cmd_resp = dsp.parse_response();
    assert_eq!(cmd_resp["id"], 402);
    assert_eq!(cmd_resp["result"]["applied"], true);
}

#[test]
fn test_execute_command_apply_edits_client_reject_aborts_edits() {
    let mut dsp = TestDispatcher::new();
    let file = std::env::temp_dir().join(format!("ridx_exec_p4_rej_{}.rs", EXEC_P4_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri = to_uri(&file);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": { "uri": &uri, "languageId": "rust", "version": 1, "text": "let x = 1;" }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let cmd = json!({
        "jsonrpc": "2.0", "id": 403, "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [
                {
                    "changes": {
                        &uri: [
                            { "range": { "start": { "line": 0, "character": 4 }, "end": { "line": 0, "character": 5 } }, "newText": "rejected_var" }
                        ]
                    }
                }
            ]
        }
    });
    assert!(dsp.dispatch(&cmd));
    let apply_req = dsp.parse_response();
    assert_eq!(apply_req["method"], "workspace/applyEdit");
    let apply_id = apply_req["id"].clone();

    let client_nack = json!({
        "jsonrpc": "2.0",
        "id": apply_id,
        "result": { "applied": false }
    });
    assert!(dsp.dispatch(&client_nack));
    let cmd_resp = dsp.parse_response();
    assert_eq!(cmd_resp["id"], 403);
    assert_eq!(cmd_resp["result"]["applied"], false);
}

#[test]
fn test_execute_command_apply_edits_two_concurrent_independent_commands() {
    let mut dsp = TestDispatcher::new();
    let file1 = std::env::temp_dir().join(format!("ridx_exec_p4_c1_{}.rs", EXEC_P4_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let file2 = std::env::temp_dir().join(format!("ridx_exec_p4_c2_{}.rs", EXEC_P4_COUNTER.fetch_add(1, Ordering::Relaxed)));
    let uri1 = to_uri(&file1);
    let uri2 = to_uri(&file2);

    let open1 = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": { "uri": &uri1, "languageId": "rust", "version": 1, "text": "let a = 1;" } }
    });
    let open2 = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": { "uri": &uri2, "languageId": "rust", "version": 1, "text": "let b = 2;" } }
    });
    assert!(dsp.dispatch(&open1));
    assert!(dsp.dispatch(&open2));

    let cmd1 = json!({
        "jsonrpc": "2.0", "id": 404, "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [{ "changes": { &uri1: [{ "range": { "start": { "line": 0, "character": 4 }, "end": { "line": 0, "character": 5 } }, "newText": "aa" }] } }]
        }
    });
    let cmd2 = json!({
        "jsonrpc": "2.0", "id": 405, "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [{ "changes": { &uri2: [{ "range": { "start": { "line": 0, "character": 4 }, "end": { "line": 0, "character": 5 } }, "newText": "bb" }] } }]
        }
    });
    assert!(dsp.dispatch(&cmd1));
    let apply_req1 = dsp.parse_response();
    assert!(dsp.dispatch(&cmd2));
    let apply_req2 = dsp.parse_response();
    let id1 = apply_req1["id"].as_u64().unwrap();
    let id2 = apply_req2["id"].as_u64().unwrap();
    assert_eq!(id2, id1 + 1);
}

#[test]
fn test_execute_command_resolve_workspace_idempotence() {
    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 406, "method": "workspace/executeCommand",
        "params": { "command": "ridx.resolveWorkspace", "rootUri": "file:///stable" }
    });
    assert!(dsp.dispatch(&req));
    let resp1 = dsp.parse_response();
    assert_eq!(resp1["id"], 406);

    assert!(dsp.dispatch(&req));
    let resp2 = dsp.parse_response();
    assert_eq!(resp2["id"], 406);
    assert_eq!(resp1["result"], resp2["result"]);
}

#[test]
fn test_execute_command_show_references_string_id_preserved() {
    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": "show-refs-id-777", "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.showReferences",
            "arguments": [
                "file:///workspace/ref.rs",
                { "line": 5, "character": 10 },
                []
            ]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], "show-refs-id-777");
    assert_eq!(resp["result"]["success"], true);
}
