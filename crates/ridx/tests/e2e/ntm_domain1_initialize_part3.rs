/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for initialize/initialized part 3 (Domain 1: Lifecycle & Control).

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
fn test_initialize_server_info_name_and_version() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 101,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    let _ = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    assert_eq!(val["id"], 101);
    let server_info = &val["result"]["serverInfo"];
    assert_eq!(server_info["name"], "ridx");
    assert_eq!(server_info["version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn test_initialize_semantic_tokens_provider_capabilities() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 102,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    let _ = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    let sem = &val["result"]["capabilities"]["semanticTokensProvider"];
    assert_eq!(sem["range"], true);
    assert_eq!(sem["full"]["delta"], true);
    assert_eq!(sem["legend"]["tokenTypes"].as_array().unwrap().len(), 20);
    assert_eq!(sem["legend"]["tokenModifiers"].as_array().unwrap().len(), 4);
}

#[test]
fn test_initialize_code_action_resolve_provider() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 103,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    let _ = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    let ca = &val["result"]["capabilities"]["codeActionProvider"];
    assert_eq!(ca["resolveProvider"], true);
}

#[test]
fn test_initialize_inlay_hint_resolve_provider() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 104,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    let _ = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    let ih = &val["result"]["capabilities"]["inlayHintProvider"];
    assert_eq!(ih["resolveProvider"], true);
}

#[test]
fn test_initialize_rename_prepare_provider() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 105,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    let _ = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    let rename = &val["result"]["capabilities"]["renameProvider"];
    assert_eq!(rename["prepareProvider"], true);
}

#[test]
fn test_initialize_completion_trigger_characters() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 106,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    let _ = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    let comp = &val["result"]["capabilities"]["completionProvider"];
    assert_eq!(comp["resolveProvider"], true);
    let triggers = comp["triggerCharacters"].as_array().expect("triggers");
    let chars: Vec<&str> = triggers.iter().filter_map(|t| t.as_str()).collect();
    assert_eq!(chars, vec![".", ":"]);
}

#[test]
fn test_initialize_diagnostic_provider_inter_file() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 107,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    let _ = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    let diag = &val["result"]["capabilities"]["diagnosticProvider"];
    assert_eq!(diag["interFileDependencies"], true);
    assert_eq!(diag["workspaceDiagnostics"], true);
}

#[test]
fn test_initialize_root_path_fallback_when_root_uri_missing() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 108,
        "method": "initialize",
        "params": {
            "rootPath": "/workspace/sample",
            "capabilities": {}
        }
    });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
    let val = extract_response(&out);
    assert_eq!(val["id"], 108);
    assert_eq!(val["result"]["capabilities"]["definitionProvider"], true);
}

#[test]
fn test_initialize_client_info_handling() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 109,
        "method": "initialize",
        "params": {
            "clientInfo": {
                "name": "Visual Studio Code",
                "version": "1.90.0"
            },
            "capabilities": {}
        }
    });
    let _ = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    assert_eq!(val["id"], 109);
    assert_eq!(val["result"]["serverInfo"]["name"], "ridx");
}

#[test]
fn test_initialize_initialized_notification_no_response() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    // 1. Initialize
    let init_msg = json!({
        "jsonrpc": "2.0",
        "id": 110,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    let _ = dispatch_client(init_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    out.clear();

    // 2. Initialized notification
    let notif = json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {}
    });
    let _ = dispatch_client(notif, &mut out, &mut mr, &mut ov, &mut lc, &can);
    // Notification produces no response body
    let val = extract_response(&out);
    assert!(val.is_null());
}
