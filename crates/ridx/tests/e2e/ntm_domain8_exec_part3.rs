/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspace/executeCommand (Domain 8, Part 3).
//! Tests unknown commands, missing fields, shutdown errors, cancellation, and isolation.

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
fn test_execute_command_unknown_command_returns_code_32602() {
    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 301, "method": "workspace/executeCommand",
        "params": {
            "command": "custom.unknownCommand"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 301);
    assert_eq!(resp["error"]["code"], -32602);
}

#[test]
fn test_execute_command_unknown_command_name_in_error_message() {
    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 302, "method": "workspace/executeCommand",
        "params": {
            "command": "some.obscure.command"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 302);
    assert_eq!(resp["error"]["code"], -32602);
    assert_eq!(resp["error"]["message"], "Unknown command: 'some.obscure.command'");
}

#[test]
fn test_execute_command_empty_command_string_returns_code_32602() {
    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 303, "method": "workspace/executeCommand",
        "params": {
            "command": ""
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 303);
    assert_eq!(resp["error"]["code"], -32602);
}

#[test]
fn test_execute_command_missing_command_field_returns_code_32602() {
    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 304, "method": "workspace/executeCommand",
        "params": {
            "arguments": []
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 304);
    assert_eq!(resp["error"]["code"], -32602);
}

#[test]
fn test_execute_command_missing_params_field_handled() {
    let mut dsp = TestDispatcher::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 305, "method": "workspace/executeCommand"
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 305);
    assert_eq!(resp["error"]["code"], -32602);
}

#[test]
fn test_execute_command_after_shutdown_returns_code_32600() {
    let mut dsp = TestDispatcher::new();
    let shutdown_req = json!({
        "jsonrpc": "2.0", "id": 399, "method": "shutdown"
    });
    assert!(dsp.dispatch(&shutdown_req));
    let shutdown_resp = dsp.parse_response();
    assert_eq!(shutdown_resp["id"], 399);

    let req = json!({
        "jsonrpc": "2.0", "id": 306, "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.resolveWorkspace"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 306);
    assert_eq!(resp["error"]["code"], -32600);
}

#[test]
fn test_execute_command_cancelled_request_returns_code_32800() {
    let mut dsp = TestDispatcher::new();
    let cancel_msg = json!({
        "jsonrpc": "2.0", "method": "$/cancelRequest",
        "params": { "id": 307 }
    });
    assert!(dsp.dispatch(&cancel_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 307, "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.resolveWorkspace"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 307);
    assert_eq!(resp["error"]["code"], -32800);
}

#[test]
fn test_execute_command_null_id_notification_handled() {
    let mut dsp = TestDispatcher::new();
    let notif = json!({
        "jsonrpc": "2.0", "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.resolveWorkspace",
            "rootUri": "file:///workspace"
        }
    });
    assert!(dsp.dispatch(&notif));
}

#[test]
fn test_execute_command_consecutive_commands_isolation() {
    let mut dsp = TestDispatcher::new();
    let req1 = json!({
        "jsonrpc": "2.0", "id": 308, "method": "workspace/executeCommand",
        "params": { "command": "ridx.resolveWorkspace", "rootUri": "file:///first" }
    });
    assert!(dsp.dispatch(&req1));
    let resp1 = dsp.parse_response();
    assert_eq!(resp1["id"], 308);
    assert_eq!(resp1["result"]["workspaceRoot"], "file:///first");

    let req2 = json!({
        "jsonrpc": "2.0", "id": 309, "method": "workspace/executeCommand",
        "params": { "command": "ridx.resolveWorkspace", "rootUri": "file:///second" }
    });
    assert!(dsp.dispatch(&req2));
    let resp2 = dsp.parse_response();
    assert_eq!(resp2["id"], 309);
    assert_eq!(resp2["result"]["workspaceRoot"], "file:///second");
}
