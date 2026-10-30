/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for $/setTrace notification part 5 (Domain 1: Lifecycle & Control).

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::{LifecycleManager, ServerState};
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
fn test_set_trace_workspace_roots_unaffected() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "off" } });
    let _ = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(mr.len(), 0);
}

#[test]
fn test_set_trace_cancelled_requests_unaffected() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    can.lock().unwrap().insert(json!(999));
    let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "verbose" } });
    let _ = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(*can.lock().unwrap(), HashSet::from([json!(999)]));
}

#[test]
fn test_set_trace_lifecycle_state_unaffected() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let state_before = lc.state();
    let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "off" } });
    let _ = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(lc.state(), state_before);
}

#[test]
fn test_set_trace_stress_rapid_fire() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    for i in 0..20 {
        let val = if i % 2 == 0 { "off" } else { "verbose" };
        let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": val } });
        assert!(dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap());
    }
    assert_eq!(out, b"");
}

#[test]
fn test_set_trace_uppercase_off_safe() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "OFF" } });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
}

#[test]
fn test_set_trace_uppercase_verbose_safe() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "VERBOSE" } });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
}

#[test]
fn test_set_trace_empty_string_value_safe() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "" } });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
}

#[test]
fn test_set_trace_whitespace_string_value_safe() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "   " } });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
}

#[test]
fn test_set_trace_without_jsonrpc_version_safe() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let msg = json!({ "method": "$/setTrace", "params": { "value": "off" } });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
}

#[test]
fn test_set_trace_final_lifecycle_checkpoint() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "messages" } });
    assert!(dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap());
    assert_eq!(lc.state(), ServerState::Initialized);
}
