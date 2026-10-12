/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for initialize request (Domain 1: Lifecycle & Control).

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::LspMessage;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use serde_json::json;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

fn extract_response(out: &[u8]) -> serde_json::Value {
    let s = String::from_utf8_lossy(out);
    for part in s.split("Content-Length: ") {
        if let Some(idx) = part.find("\r\n\r\n") {
            let body = &part[idx + 4..];
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(body.trim()) {
                return v;
            }
        }
    }
    serde_json::Value::Null
}

#[test]
fn test_initialize_basic_root_uri() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "rootUri": "file:///tmp/dummy_ws",
            "capabilities": {}
        }
    });

    let res = dispatch_lsp_message(LspMessage::Client(msg), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
    assert!(res);
    assert!(lc.is_initialized());

    let resp = extract_response(&out);
    assert_eq!(resp["id"], 1);
    assert_eq!(resp["result"]["serverInfo"]["name"], "ridx");
    assert_eq!(resp["result"]["serverInfo"]["version"], "0.1.0");
}

#[test]
fn test_initialize_basic_workspace_folders() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "initialize",
        "params": {
            "workspaceFolders": [
                { "uri": "file:///tmp/dummy_ws1", "name": "ws1" },
                { "uri": "file:///tmp/dummy_ws2", "name": "ws2" }
            ],
            "capabilities": {}
        }
    });

    let res = dispatch_lsp_message(LspMessage::Client(msg), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
    assert!(res);
    assert!(lc.is_initialized());
    let resp = extract_response(&out);
    assert_eq!(resp["id"], 2);
    assert_eq!(resp["result"]["capabilities"]["workspace"]["workspaceFolders"]["supported"], true);
}

#[test]
fn test_initialize_fallback_to_root_path() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "initialize",
        "params": {
            "rootPath": "/tmp/dummy_ws",
            "capabilities": {}
        }
    });

    let res = dispatch_lsp_message(LspMessage::Client(msg), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
    assert!(res);
    assert!(lc.is_initialized());
    let resp = extract_response(&out);
    assert_eq!(resp["id"], 3);
}

#[test]
fn test_initialize_empty_params() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "initialize",
        "params": {}
    });

    let res = dispatch_lsp_message(LspMessage::Client(msg), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
    assert!(res);
    assert!(lc.is_initialized());
    let resp = extract_response(&out);
    assert_eq!(resp["id"], 4);
    assert_eq!(resp["result"]["capabilities"]["definitionProvider"], true);
    assert_eq!(resp["result"]["capabilities"]["textDocumentSync"]["openClose"], true);
}

#[test]
fn test_initialize_client_capabilities_pull_diagnostic_enabled() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "initialize",
        "params": {
            "capabilities": {
                "textDocument": {
                    "diagnostic": { "dynamicRegistration": true }
                }
            }
        }
    });

    let _ = dispatch_lsp_message(LspMessage::Client(msg), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
    assert!(lc.client_pull_diagnostics());
    assert!(!lc.client_refresh_diagnostics());
}

#[test]
fn test_initialize_client_capabilities_pull_diagnostic_disabled() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 6,
        "method": "initialize",
        "params": {
            "capabilities": {
                "textDocument": {}
            }
        }
    });

    let _ = dispatch_lsp_message(LspMessage::Client(msg), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
    assert!(!lc.client_pull_diagnostics());
    assert!(!lc.client_refresh_diagnostics());
}

#[test]
fn test_initialize_client_capabilities_refresh_support_true() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "initialize",
        "params": {
            "capabilities": {
                "workspace": {
                    "diagnostics": { "refreshSupport": true }
                }
            }
        }
    });

    let _ = dispatch_lsp_message(LspMessage::Client(msg), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
    assert!(lc.client_refresh_diagnostics());
}

#[test]
fn test_initialize_client_capabilities_refresh_support_false() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 8,
        "method": "initialize",
        "params": {
            "capabilities": {
                "workspace": {
                    "diagnostics": { "refreshSupport": false }
                }
            }
        }
    });

    let _ = dispatch_lsp_message(LspMessage::Client(msg), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
    assert!(!lc.client_refresh_diagnostics());
}

#[test]
fn test_initialize_text_document_sync_capabilities() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 9,
        "method": "initialize",
        "params": { "capabilities": {} }
    });

    let _ = dispatch_lsp_message(LspMessage::Client(msg), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
    let resp = extract_response(&out);
    let sync = &resp["result"]["capabilities"]["textDocumentSync"];
    assert_eq!(sync["openClose"], true);
    assert_eq!(sync["change"], 2);
    assert_eq!(sync["willSave"], true);
    assert_eq!(sync["willSaveWaitUntil"], true);
    assert_eq!(sync["save"]["includeText"], false);
}

#[test]
fn test_initialize_file_operations_capabilities() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 10,
        "method": "initialize",
        "params": { "capabilities": {} }
    });

    let _ = dispatch_lsp_message(LspMessage::Client(msg), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
    let resp = extract_response(&out);
    let ops = &resp["result"]["capabilities"]["workspace"]["fileOperations"];
    assert_eq!(ops["didCreate"]["filters"][0]["pattern"]["glob"], "**/*.rs");
    assert_eq!(ops["didRename"]["filters"][0]["pattern"]["glob"], "**/*.rs");
    assert_eq!(ops["didDelete"]["filters"][0]["pattern"]["glob"], "**/*.rs");
}
