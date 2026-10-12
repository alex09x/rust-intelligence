/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for initialize/initialized part 4 (Domain 1: Lifecycle & Control).

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
fn test_initialize_capabilities_type_definition_provider() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 111,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    let _ = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    assert_eq!(val["result"]["capabilities"]["typeDefinitionProvider"], true);
}

#[test]
fn test_initialize_capabilities_implementation_provider() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 112,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    let _ = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    assert_eq!(val["result"]["capabilities"]["implementationProvider"], true);
}

#[test]
fn test_initialize_capabilities_declaration_provider() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 113,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    let _ = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    assert_eq!(val["result"]["capabilities"]["declarationProvider"], true);
}

#[test]
fn test_initialize_capabilities_call_hierarchy_provider() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 114,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    let _ = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    assert_eq!(val["result"]["capabilities"]["callHierarchyProvider"], true);
}

#[test]
fn test_initialize_capabilities_type_hierarchy_provider() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 115,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    let _ = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    assert_eq!(val["result"]["capabilities"]["typeHierarchyProvider"], true);
}

#[test]
fn test_initialize_capabilities_document_formatting_provider() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 116,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    let _ = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    assert_eq!(val["result"]["capabilities"]["documentFormattingProvider"], true);
}

#[test]
fn test_initialize_capabilities_document_range_formatting_provider() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 117,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    let _ = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    assert_eq!(val["result"]["capabilities"]["documentRangeFormattingProvider"], true);
}

#[test]
fn test_initialize_capabilities_document_highlight_provider() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 118,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    let _ = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    assert_eq!(val["result"]["capabilities"]["documentHighlightProvider"], true);
}

#[test]
fn test_initialize_capabilities_folding_range_provider() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 119,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    let _ = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    assert_eq!(val["result"]["capabilities"]["foldingRangeProvider"], true);
}

#[test]
fn test_initialize_capabilities_selection_range_provider() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({
        "jsonrpc": "2.0",
        "id": 120,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    let _ = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let val = extract_response(&out);
    assert_eq!(val["result"]["capabilities"]["selectionRangeProvider"], true);
}
