/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for initialize/initialized part 2 (Domain 1: Lifecycle & Control).

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
fn test_initialize_already_initialized_rejected() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg1 = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    let res1 = dispatch_client(msg1, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res1.unwrap(), true);

    out.clear();
    let msg2 = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    let res2 = dispatch_client(msg2, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res2.unwrap(), true);

    let val = extract_response(&out);
    assert_eq!(val["id"], 2);
    assert_eq!(val["error"]["code"], -32600);
}

#[test]
fn test_initialize_capabilities_hover_provider() {
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
    dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    let val = extract_response(&out);
    assert_eq!(val["result"]["capabilities"]["hoverProvider"], true);
    assert_eq!(val["result"]["capabilities"]["definitionProvider"], true);
    assert_eq!(val["result"]["capabilities"]["referencesProvider"], true);
}

#[test]
fn test_initialize_semantic_tokens_legend() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 20,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    let val = extract_response(&out);
    let legend = &val["result"]["capabilities"]["semanticTokensProvider"]["legend"];
    let types = legend["tokenTypes"].as_array().unwrap();
    let mods = legend["tokenModifiers"].as_array().unwrap();
    assert_eq!(types.len(), 18);
    assert_eq!(mods.len(), 4);
    assert_eq!(types[0], "namespace");
    assert_eq!(mods[0], "declaration");
}

#[test]
fn test_initialize_completion_trigger_chars() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 30,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    let val = extract_response(&out);
    let comp = &val["result"]["capabilities"]["completionProvider"];
    let triggers = comp["triggerCharacters"].as_array().unwrap();
    assert_eq!(triggers.len(), 2);
    assert_eq!(triggers[0], ".");
    assert_eq!(triggers[1], ":");
    assert_eq!(comp["resolveProvider"], true);
}

#[test]
fn test_initialize_signature_help_trigger_chars() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 40,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    let val = extract_response(&out);
    let sig = &val["result"]["capabilities"]["signatureHelpProvider"];
    let triggers = sig["triggerCharacters"].as_array().unwrap();
    assert_eq!(triggers.len(), 2);
    assert_eq!(triggers[0], "(");
    assert_eq!(triggers[1], ",");
}

#[test]
fn test_initialize_formatting_trigger_chars() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 50,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    let val = extract_response(&out);
    let ont = &val["result"]["capabilities"]["documentOnTypeFormattingProvider"];
    assert_eq!(ont["firstTriggerCharacter"], ";");
    let more = ont["moreTriggerCharacter"].as_array().unwrap();
    assert_eq!(more.len(), 1);
    assert_eq!(more[0], "}");
}

#[test]
fn test_initialize_code_action_kinds() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 60,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    let val = extract_response(&out);
    let ca = &val["result"]["capabilities"]["codeActionProvider"];
    let kinds = ca["codeActionKinds"].as_array().unwrap();
    assert_eq!(kinds.len(), 3);
    assert_eq!(kinds[0], "quickfix");
    assert_eq!(kinds[1], "refactor");
    assert_eq!(kinds[2], "source.organizeImports");
    assert_eq!(ca["resolveProvider"], true);
}

#[test]
fn test_initialized_notification_handling() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let init_msg = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    dispatch_client(init_msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    out.clear();

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {}
    });
    let res = dispatch_client(notif, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    assert!(res);
    assert_eq!(out, b"");
}

#[test]
fn test_initialized_duplicate_notification_handling() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {}
    });
    let res1 = dispatch_client(notif.clone(), &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    assert!(res1);
    let res2 = dispatch_client(notif, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    assert!(res2);
    assert_eq!(out, b"");
}

#[test]
fn test_initialize_server_info_exact() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 99,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    let val = extract_response(&out);
    assert_eq!(val["result"]["serverInfo"]["name"], "ridx");
    assert_eq!(val["result"]["serverInfo"]["version"], "0.1.0");
}
