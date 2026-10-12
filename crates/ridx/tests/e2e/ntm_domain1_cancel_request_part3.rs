/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for $/cancelRequest notification part 3 (Domain 1: Lifecycle & Control).

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::LspMessage;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use serde_json::json;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

fn setup_initialized_server() -> (
    MultiRootWorkspace,
    OverlayManager,
    LifecycleManager,
    Arc<Mutex<HashSet<serde_json::Value>>>,
) {
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
fn test_cancel_request_string_id_recorded_in_cancelled_set() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let cancel_msg = json!({
        "jsonrpc": "2.0",
        "method": "$/cancelRequest",
        "params": { "id": "req-token-xyz" }
    });
    let res = dispatch_client(cancel_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
    assert_eq!(*can.lock().unwrap(), HashSet::from([json!("req-token-xyz")]));
}

#[test]
fn test_cancel_request_numeric_id_recorded_in_cancelled_set() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let cancel_msg = json!({
        "jsonrpc": "2.0",
        "method": "$/cancelRequest",
        "params": { "id": 4242 }
    });
    let res = dispatch_client(cancel_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
    assert_eq!(*can.lock().unwrap(), HashSet::from([json!(4242)]));
}

#[test]
fn test_cancel_request_large_integer_id_recorded() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let cancel_msg = json!({
        "jsonrpc": "2.0",
        "method": "$/cancelRequest",
        "params": { "id": 9_007_199_254_740_991_i64 }
    });
    let res = dispatch_client(cancel_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
    assert_eq!(*can.lock().unwrap(), HashSet::from([json!(9_007_199_254_740_991_i64)]));
}

#[test]
fn test_cancel_request_negative_id_recorded() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let cancel_msg = json!({
        "jsonrpc": "2.0",
        "method": "$/cancelRequest",
        "params": { "id": -50 }
    });
    let res = dispatch_client(cancel_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
    assert_eq!(*can.lock().unwrap(), HashSet::from([json!(-50)]));
}

#[test]
fn test_cancel_request_produces_no_response_bytes() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let cancel_msg = json!({
        "jsonrpc": "2.0",
        "method": "$/cancelRequest",
        "params": { "id": 10 }
    });
    let _ = dispatch_client(cancel_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(out, b"");
}

#[test]
fn test_cancel_request_duplicate_cancel_idempotent() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let cancel_msg1 = json!({ "jsonrpc": "2.0", "method": "$/cancelRequest", "params": { "id": 11 } });
    let cancel_msg2 = json!({ "jsonrpc": "2.0", "method": "$/cancelRequest", "params": { "id": 11 } });

    assert!(dispatch_client(cancel_msg1, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap());
    assert!(dispatch_client(cancel_msg2, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap());
    assert_eq!(*can.lock().unwrap(), HashSet::from([json!(11)]));
}

#[test]
fn test_cancel_request_null_params_handled_safely() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let cancel_msg = json!({
        "jsonrpc": "2.0",
        "method": "$/cancelRequest",
        "params": null
    });
    let res = dispatch_client(cancel_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
}

#[test]
fn test_cancel_request_missing_id_in_params_handled_safely() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let cancel_msg = json!({
        "jsonrpc": "2.0",
        "method": "$/cancelRequest",
        "params": { "other_field": true }
    });
    let res = dispatch_client(cancel_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
}

#[test]
fn test_cancel_request_empty_params_handled_safely() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();

    let cancel_msg = json!({
        "jsonrpc": "2.0",
        "method": "$/cancelRequest",
        "params": {}
    });
    let res = dispatch_client(cancel_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
}

#[test]
fn test_cancel_request_uninitialized_server_handles_safely() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new(); // uninitialized
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let cancel_msg = json!({
        "jsonrpc": "2.0",
        "method": "$/cancelRequest",
        "params": { "id": "pre-init-token" }
    });
    let res = dispatch_client(cancel_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res.unwrap());
    assert_eq!(*can.lock().unwrap(), HashSet::from([json!("pre-init-token")]));
}
