/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::LspMessage;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

fn extract_published_diagnostics(output: &[u8]) -> Vec<Value> {
    let mut diags = Vec::new();
    let text = String::from_utf8_lossy(output);
    for part in text.split("Content-Length: ") {
        if let Some(idx) = part.find("\r\n\r\n") {
            let json_str = &part[idx + 4..];
            if let Ok(val) = serde_json::from_str::<Value>(json_str.trim()) {
                if val["method"] == "textDocument/publishDiagnostics" {
                    diags.push(val);
                }
            }
        }
    }
    diags
}

fn extract_messages_by_method(output: &[u8], method: &str) -> Vec<Value> {
    let mut msgs = Vec::new();
    let text = String::from_utf8_lossy(output);
    for part in text.split("Content-Length: ") {
        if let Some(idx) = part.find("\r\n\r\n") {
            let json_str = &part[idx + 4..];
            if let Ok(val) = serde_json::from_str::<Value>(json_str.trim()) {
                if val["method"] == method {
                    msgs.push(val);
                }
            }
        }
    }
    msgs
}

#[test]
fn test_e2e_pull_diagnostic_client_no_publish_on_did_change() {
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let mut lifecycle = LifecycleManager::new();
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    // 1. Initialize with pull diagnostic capability
    let init_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "rootUri": "file:///tmp/dummy_workspace",
            "capabilities": {
                "textDocument": {
                    "diagnostic": { "dynamicRegistration": true }
                },
                "workspace": {
                    "diagnostics": {
                        "refreshSupport": true
                    }
                }
            }
        }
    });
    let mut out = Vec::new();
    dispatch_lsp_message(LspMessage::Client(init_req), &mut out, &mut lifecycle, &mut multi_root, &mut overlay_mgr, &cancelled, None).unwrap();

    assert!(lifecycle.client_pull_diagnostics(), "Must recognize pull diagnostic client");
    assert!(lifecycle.client_refresh_diagnostics(), "Must recognize workspace diagnostic refresh support");

    let out_str = String::from_utf8_lossy(&out);
    assert!(out_str.contains("diagnosticProvider"), "Must advertise diagnosticProvider in initialize result");

    let uri = "file:///tmp/dummy_workspace/src/lib.rs";

    // 2. didOpen
    out.clear();
    let open_notif = json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": uri,
                "languageId": "rust",
                "version": 1,
                "text": "fn foo() { let x = 1; }\n"
            }
        }
    });
    dispatch_lsp_message(LspMessage::Client(open_notif), &mut out, &mut lifecycle, &mut multi_root, &mut overlay_mgr, &cancelled, None).unwrap();

    // 3. didChange with unclosed delimiter error
    out.clear();
    let change_notif = json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didChange",
        "params": {
            "textDocument": {
                "uri": uri,
                "version": 2
            },
            "contentChanges": [
                { "text": "fn foo() { let x = 1; \n" }
            ]
        }
    });
    dispatch_lsp_message(LspMessage::Client(change_notif), &mut out, &mut lifecycle, &mut multi_root, &mut overlay_mgr, &cancelled, None).unwrap();

    // Verify: Pull client receives NO push publishDiagnostics on didChange!
    let published = extract_published_diagnostics(&out);
    assert!(published.is_empty(), "Pull clients must not receive push publishDiagnostics on didChange");

    // 4. Client pulls textDocument/diagnostic
    out.clear();
    let pull_req = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "textDocument/diagnostic",
        "params": {
            "textDocument": { "uri": uri }
        }
    });
    dispatch_lsp_message(LspMessage::Client(pull_req), &mut out, &mut lifecycle, &mut multi_root, &mut overlay_mgr, &cancelled, None).unwrap();

    let pull_str = String::from_utf8_lossy(&out);
    assert!(pull_str.contains("\"kind\":\"full\""));
    assert!(pull_str.contains("unclosed delimiter"), "Pull response must contain the syntax diagnostic");

    // 5. InternalDiagnostic from flycheck triggers workspace/diagnostic/refresh with unique ID
    out.clear();
    let gen = lifecycle.flycheck.current_generation();
    let flycheck_diag = json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": {
            "uri": uri,
            "diagnostics": [
                {
                    "range": {
                        "start": { "line": 0, "character": 0 },
                        "end": { "line": 0, "character": 2 }
                    },
                    "severity": 1,
                    "code": "E0425",
                    "source": "rustc",
                    "message": "compiler error"
                }
            ]
        }
    });
    dispatch_lsp_message(LspMessage::InternalDiagnostic(gen, flycheck_diag), &mut out, &mut lifecycle, &mut multi_root, &mut overlay_mgr, &cancelled, None).unwrap();

    let refresh_msgs = extract_messages_by_method(&out, "workspace/diagnostic/refresh");
    assert_eq!(refresh_msgs.len(), 1, "Must send workspace/diagnostic/refresh for pull clients on flycheck completion");
    assert_eq!(refresh_msgs[0]["id"], "ridx-refresh-1");
    let published_after_flycheck = extract_published_diagnostics(&out);
    assert!(published_after_flycheck.is_empty(), "Must not send push publishDiagnostics to pull client after flycheck");

    // 6. Client responds to server-initiated workspace/diagnostic/refresh request
    out.clear();
    let client_response = json!({
        "jsonrpc": "2.0",
        "id": "ridx-refresh-1",
        "result": null
    });
    dispatch_lsp_message(LspMessage::Client(client_response), &mut out, &mut lifecycle, &mut multi_root, &mut overlay_mgr, &cancelled, None).unwrap();
    assert!(out.is_empty(), "Client response without method must be silently accepted without errors or output");
}

#[test]
fn test_e2e_pull_client_without_refresh_support_no_server_refresh_request() {
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let mut lifecycle = LifecycleManager::new();
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    // Initialize with diagnostic pull support but NO refreshSupport
    let init_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "rootUri": "file:///tmp/dummy_workspace",
            "capabilities": {
                "textDocument": {
                    "diagnostic": { "dynamicRegistration": true }
                }
            }
        }
    });
    let mut out = Vec::new();
    dispatch_lsp_message(LspMessage::Client(init_req), &mut out, &mut lifecycle, &mut multi_root, &mut overlay_mgr, &cancelled, None).unwrap();

    assert!(lifecycle.client_pull_diagnostics());
    assert!(!lifecycle.client_refresh_diagnostics());

    // InternalDiagnostic should not trigger refresh request when refreshSupport is false
    out.clear();
    let gen = lifecycle.flycheck.current_generation();
    let flycheck_diag = json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": {
            "uri": "file:///tmp/dummy_workspace/src/lib.rs",
            "diagnostics": []
        }
    });
    dispatch_lsp_message(LspMessage::InternalDiagnostic(gen, flycheck_diag), &mut out, &mut lifecycle, &mut multi_root, &mut overlay_mgr, &cancelled, None).unwrap();

    let refresh_msgs = extract_messages_by_method(&out, "workspace/diagnostic/refresh");
    assert!(refresh_msgs.is_empty(), "Must NOT send workspace/diagnostic/refresh if client lacks refreshSupport");
}
