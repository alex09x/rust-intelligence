/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for exit notification (Domain 1: Lifecycle & Control).

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
fn test_exit_after_shutdown_returns_ok() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } });
    dispatch_client(init, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    let shut = json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" });
    dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    let exit_msg = json!({ "jsonrpc": "2.0", "method": "exit" });
    let res = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(!res.unwrap());
    assert!(lc.is_shutdown());
}

#[test]
fn test_exit_before_shutdown_returns_err() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } });
    dispatch_client(init, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    let exit_msg = json!({ "jsonrpc": "2.0", "method": "exit" });
    let res = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let err = res.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::Other);
    assert_eq!(err.to_string(), "exit notification received before shutdown");
}

#[test]
fn test_exit_from_uninitialized_returns_err() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let exit_msg = json!({ "jsonrpc": "2.0", "method": "exit" });
    let res = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let err = res.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::Other);
    assert_eq!(err.to_string(), "exit notification received before shutdown");
}

#[test]
fn test_exit_from_initialized_returns_err() {
    let mut lc = LifecycleManager::new();
    let init = json!({ "params": { "capabilities": {} } });
    lc.handle_initialize(&init).unwrap();
    let err = lc.handle_exit().unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::Other);
    assert_eq!(err.to_string(), "exit notification received before shutdown");
}

#[test]
fn test_exit_handle_exit_shutdown_parity() {
    let mut lc = LifecycleManager::new();
    let err = lc.handle_exit().unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::Other);
    assert_eq!(err.to_string(), "exit notification received before shutdown");
    let init = json!({ "params": { "capabilities": {} } });
    lc.handle_initialize(&init).unwrap();
    let err = lc.handle_exit().unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::Other);
    assert_eq!(err.to_string(), "exit notification received before shutdown");
    lc.handle_shutdown().unwrap();
    assert_eq!(lc.handle_exit().unwrap(), ());
}

#[test]
fn test_exit_check_lifecycle_and_cancel_returns_false() {
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let init = json!({ "params": { "capabilities": {} } });
    lc.handle_initialize(&init).unwrap();
    lc.handle_shutdown().unwrap();

    let res = lc.check_lifecycle_and_cancel("exit", &None, &can, &mut out);
    assert_eq!(res.unwrap(), Some(false));
}

#[test]
fn test_exit_empty_params_tolerance() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } });
    dispatch_client(init, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    let shut = json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" });
    dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    let exit_msg = json!({ "jsonrpc": "2.0", "method": "exit", "params": {} });
    let res = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(!res.unwrap());
}

#[test]
fn test_exit_null_params_tolerance() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } });
    dispatch_client(init, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    let shut = json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" });
    dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    let exit_msg = json!({ "jsonrpc": "2.0", "method": "exit", "params": null });
    let res = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(!res.unwrap());
}

#[test]
fn test_exit_missing_params_tolerance() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let init = json!({ "jsonrpc": "2.0", "id": 10, "method": "initialize", "params": { "capabilities": {} } });
    dispatch_client(init, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    let shut = json!({ "jsonrpc": "2.0", "id": 20, "method": "shutdown" });
    dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    // Notification without params key
    let exit_msg = json!({ "jsonrpc": "2.0", "method": "exit" });
    let res = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(!res.unwrap());
    assert_eq!(*can.lock().unwrap(), HashSet::new());
}

#[test]
fn test_exit_ignores_extra_params() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } });
    dispatch_client(init, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    let shut = json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" });
    dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    let exit_msg = json!({ "jsonrpc": "2.0", "method": "exit", "params": { "custom": "field", "code": 0 } });
    let res = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(!res.unwrap());
}
