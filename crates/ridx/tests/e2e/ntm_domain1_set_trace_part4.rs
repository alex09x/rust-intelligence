/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for $/setTrace notification part 4 (Domain 1: Lifecycle & Control).

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::LspMessage;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use serde_json::json;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

fn setup_initialized_server() -> (MultiRootWorkspace, OverlayManager, LifecycleManager, Arc<Mutex<HashSet<serde_json::Value>>>) {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let init = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    dispatch_lsp_message(LspMessage::Client(init), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
    (mr, ov, lc, can)
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
fn test_set_trace_pre_initialize_handling() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new(); // uninitialized
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "verbose" } });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
    assert_eq!(out, b"");
}

#[test]
fn test_set_trace_after_shutdown_handling() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let shut = json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" });
    let _ = dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can);
    out.clear();

    let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "off" } });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
    assert_eq!(out, b"");
}

#[test]
fn test_set_trace_interleaved_between_edits() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    ov.open_or_update("file:///lib.rs", "/lib.rs", 1, "fn a() {}".to_string());
    let tr = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "messages" } });
    assert!(dispatch_client(tr, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap());
    ov.open_or_update("file:///lib.rs", "/lib.rs", 2, "fn b() {}".to_string());
    assert_eq!(ov.get("/lib.rs").unwrap().text, "fn b() {}");
}

#[test]
fn test_set_trace_multiple_consecutive_updates() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    for level in &["off", "messages", "verbose", "off"] {
        let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": level } });
        assert!(dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap());
    }
    assert_eq!(out, b"");
}

#[test]
fn test_set_trace_toggle_off_to_verbose() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let m1 = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "off" } });
    assert!(dispatch_client(m1, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap());
    let m2 = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "verbose" } });
    assert!(dispatch_client(m2, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap());
}

#[test]
fn test_set_trace_toggle_verbose_to_messages() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let m1 = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "verbose" } });
    assert!(dispatch_client(m1, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap());
    let m2 = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "messages" } });
    assert!(dispatch_client(m2, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap());
}

#[test]
fn test_set_trace_toggle_messages_to_off() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let m1 = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "messages" } });
    assert!(dispatch_client(m1, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap());
    let m2 = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "off" } });
    assert!(dispatch_client(m2, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap());
}

#[test]
fn test_set_trace_with_erroneous_id_field() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let msg = json!({ "jsonrpc": "2.0", "id": 7777, "method": "$/setTrace", "params": { "value": "off" } });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
}

#[test]
fn test_set_trace_with_nested_object_value() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": { "level": "verbose" } } });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
}

#[test]
fn test_set_trace_overlay_manager_unaffected() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    ov.open_or_update("file:///test.rs", "/test.rs", 1, "pub struct S;".to_string());
    let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "off" } });
    let _ = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let entry = ov.get("/test.rs").unwrap();
    assert_eq!(entry.version, 1);
    assert_eq!(entry.text, "pub struct S;");
}
