/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for $/setTrace notification (Domain 1: Lifecycle & Control).

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
    dispatch_client(init, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
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
fn test_set_trace_off_returns_ok_true() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "off" } });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
    assert_eq!(out, b"");
}

#[test]
fn test_set_trace_messages_returns_ok_true() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "messages" } });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
    assert_eq!(out, b"");
}

#[test]
fn test_set_trace_verbose_returns_ok_true() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "verbose" } });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
    assert_eq!(out, b"");
}

#[test]
fn test_set_trace_produces_no_stdout() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "verbose" } });
    dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    assert_eq!(out, b"");
}

#[test]
fn test_set_trace_missing_value_tolerated() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": {} });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
    assert_eq!(out, b"");
}

#[test]
fn test_set_trace_missing_params_tolerated() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace" });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
    assert_eq!(out, b"");
}

#[test]
fn test_set_trace_unknown_value_tolerated() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "custom-tracing-level" } });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
    assert_eq!(out, b"");
}

#[test]
fn test_set_trace_pre_initialize_allowed() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "off" } });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
    assert_eq!(out, b"");
}

#[test]
fn test_set_trace_post_initialize_allowed() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let notif = json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} });
    dispatch_client(notif, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "verbose" } });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
    assert_eq!(out, b"");
    assert!(lc.is_initialized());
}

#[test]
fn test_set_trace_post_shutdown_allowed() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let shut = json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" });
    dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    out.clear();

    let msg = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "off" } });
    let res = dispatch_client(msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
    assert_eq!(out, b"");
}
