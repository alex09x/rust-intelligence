/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for exit notification part 4 (Domain 1: Lifecycle & Control).

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::LspMessage;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use serde_json::json;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

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
fn test_exit_overlay_manager_clean_after_exit() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    ov.open_or_update("file:///lib.rs", "/lib.rs", 1, "fn main() {}".to_string());
    let exit_msg = json!({ "jsonrpc": "2.0", "method": "exit" });
    let _ = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let entry = ov.get("/lib.rs").unwrap();
    assert_eq!(entry.version, 1);
    assert_eq!(entry.text, "fn main() {}");
}

#[test]
fn test_exit_cancelled_requests_set_unaffected() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    can.lock().unwrap().insert(json!(1234));
    let exit_msg = json!({ "jsonrpc": "2.0", "method": "exit" });
    let _ = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(*can.lock().unwrap(), HashSet::from([json!(1234)]));
}

#[test]
fn test_exit_after_failed_request_lifecycle() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } });
    let _ = dispatch_client(init, &mut out, &mut mr, &mut ov, &mut lc, &can);
    out.clear();

    // Bad request
    let bad = json!({ "jsonrpc": "2.0", "id": 2, "method": "unknown/method", "params": {} });
    let _ = dispatch_client(bad, &mut out, &mut mr, &mut ov, &mut lc, &can);
    out.clear();

    let shut = json!({ "jsonrpc": "2.0", "id": 3, "method": "shutdown" });
    let _ = dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can);
    out.clear();

    let exit_msg = json!({ "jsonrpc": "2.0", "method": "exit" });
    let res = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(!res.unwrap());
}

#[test]
fn test_exit_dispatch_multiple_times() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    lc.handle_shutdown().unwrap();
    let exit1 = json!({ "jsonrpc": "2.0", "method": "exit" });
    assert!(!dispatch_client(exit1, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap());

    let exit2 = json!({ "jsonrpc": "2.0", "method": "exit" });
    assert!(!dispatch_client(exit2, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap());
}

#[test]
fn test_exit_with_unexpected_id_field() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    // Client erroneously sends id with exit notification
    lc.handle_shutdown().unwrap();
    let exit_with_id = json!({ "jsonrpc": "2.0", "id": 9999, "method": "exit" });
    let res = dispatch_client(exit_with_id, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(!res.unwrap());
}

#[test]
fn test_exit_interleaved_with_custom_notifications() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    lc.handle_shutdown().unwrap();
    let set_tr = json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "off" } });
    let _ = dispatch_client(set_tr, &mut out, &mut mr, &mut ov, &mut lc, &can);

    let exit_msg = json!({ "jsonrpc": "2.0", "method": "exit" });
    let res = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(!res.unwrap());
}

#[test]
fn test_exit_idempotent_exit_code_check() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let init = json!({ "params": { "capabilities": {} } });
    lc.handle_initialize(&init).unwrap();
    lc.handle_shutdown().unwrap();

    let exit_msg = json!({ "jsonrpc": "2.0", "method": "exit" });
    assert!(!dispatch_client(exit_msg.clone(), &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap());
    assert!(!dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap());
    assert_eq!(out, b"");
}

#[test]
fn test_exit_during_active_cancelled_set() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    lc.handle_shutdown().unwrap();
    can.lock().unwrap().insert(json!("cancel-token-1"));
    let exit_msg = json!({ "jsonrpc": "2.0", "method": "exit" });
    let res = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(!res.unwrap());
}

#[test]
fn test_exit_from_shutting_down_server_lifecycle() {
    let mut lc = LifecycleManager::new();
    let init = json!({ "params": { "capabilities": {} } });
    lc.handle_initialize(&init).unwrap();
    assert_eq!(lc.state(), ridx::lsp::lifecycle::ServerState::Initialized);
    assert!(!lc.is_shutdown());
    lc.handle_shutdown().unwrap();
    assert_eq!(lc.state(), ridx::lsp::lifecycle::ServerState::ShutDown);
    assert!(lc.is_shutdown());
    assert_eq!(lc.handle_exit().unwrap(), ());
    assert!(lc.is_shutdown());
}

#[test]
fn test_exit_after_multiple_shutdown_calls_returns_exit_code_0() {
    let mut lc = LifecycleManager::new();
    let init = json!({ "params": { "capabilities": {} } });
    lc.handle_initialize(&init).unwrap();
    lc.handle_shutdown().unwrap();
    let err = lc.handle_shutdown().unwrap_err();
    assert_eq!(err.code, -32600);
    assert_eq!(err.message, "Server is already shut down");
    assert_eq!(lc.handle_exit().unwrap(), ());
}
