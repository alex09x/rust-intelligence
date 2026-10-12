/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for initialize/initialized part 5 (Domain 1: Lifecycle & Control).

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

fn dispatch_client<W: std::io::Write>(
    msg: serde_json::Value,
    out: &mut W,
    mr: &mut MultiRootWorkspace,
    ov: &mut OverlayManager,
    lc: &mut LifecycleManager,
    can: &Arc<Mutex<HashSet<serde_json::Value>>>,
) -> std::io::Result<bool> {
    dispatch_lsp_message(LspMessage::Client(msg), out, lc, mr, ov, can, None)
}

#[test]
fn test_initialize_capabilities_execute_command_provider() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 121,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    let _ = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    let exec = &val["result"]["capabilities"]["executeCommandProvider"];
    assert_eq!(exec["commands"].as_array().unwrap().len(), 3);
}

#[test]
fn test_initialize_capabilities_workspace_folders_supported() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 122,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    let _ = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    let ws = &val["result"]["capabilities"]["workspace"]["workspaceFolders"];
    assert_eq!(ws["supported"], true);
}

#[test]
fn test_initialize_with_workspace_folders_param() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 123,
        "method": "initialize",
        "params": {
            "workspaceFolders": [
                {
                    "uri": "file:///workspace/root_a",
                    "name": "root_a"
                },
                {
                    "uri": "file:///workspace/root_b",
                    "name": "root_b"
                }
            ],
            "capabilities": {}
        }
    });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
    let val = extract_response(&out);
    assert_eq!(val["result"]["capabilities"]["referencesProvider"], true);
}

#[test]
fn test_initialize_response_jsonrpc_version_2_0() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 124,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    let _ = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    assert_eq!(val["jsonrpc"], "2.0");
}

#[test]
fn test_initialize_string_id_roundtrip() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": "init-req-999",
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    let _ = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    assert_eq!(val["id"], "init-req-999");
}

#[test]
fn test_initialize_multiple_consecutive_initialized_notifications() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let init_msg = json!({
        "jsonrpc": "2.0",
        "id": 125,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    let _ = dispatch_client(init_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    out.clear();

    // Send initialized twice
    let notif1 = json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} });
    let res1 = dispatch_client(notif1, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res1.unwrap());

    let notif2 = json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} });
    let res2 = dispatch_client(notif2, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res2.unwrap());
}

#[test]
fn test_initialize_initialization_options_ignored_safely() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 126,
        "method": "initialize",
        "params": {
            "initializationOptions": {
                "checkOnSave": true,
                "cargo": { "allFeatures": true }
            },
            "capabilities": {}
        }
    });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
    let val = extract_response(&out);
    assert_eq!(val["id"], 126);
    assert_eq!(val["result"]["serverInfo"]["name"], "ridx");
}

#[test]
fn test_initialize_trace_param_handling() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 127,
        "method": "initialize",
        "params": {
            "trace": "verbose",
            "capabilities": {}
        }
    });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
    let val = extract_response(&out);
    assert_eq!(val["id"], 127);
    assert_eq!(val["result"]["serverInfo"]["name"], "ridx");
}

#[test]
fn test_initialize_null_root_uri_and_null_root_path() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 128,
        "method": "initialize",
        "params": {
            "rootUri": null,
            "rootPath": null,
            "capabilities": {}
        }
    });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
    let val = extract_response(&out);
    assert_eq!(val["id"], 128);
    assert_eq!(val["result"]["capabilities"]["hoverProvider"], true);
}

#[test]
fn test_initialize_capabilities_text_document_sync() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 129,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    let _ = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    // TextDocumentSyncKind::Incremental is 2
    assert_eq!(val["result"]["capabilities"]["textDocumentSync"]["change"], 2);
}
