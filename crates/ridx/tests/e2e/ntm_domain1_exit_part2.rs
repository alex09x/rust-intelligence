/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for exit notification part 2 (Domain 1: Lifecycle & Control).

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
fn test_exit_repeated_exit_calls() {
    let mut lc = LifecycleManager::new();
    let init = json!({ "params": { "capabilities": {} } });
    lc.handle_initialize(&init).unwrap();
    lc.handle_shutdown().unwrap();

    assert_eq!(lc.handle_exit().unwrap(), ());
    assert_eq!(lc.handle_exit().unwrap(), ());
}

#[test]
fn test_exit_full_lifecycle_sequence() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    // 1. Initialize
    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } });
    let res_init = dispatch_client(init, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res_init.unwrap());

    // 2. Initialized
    let inited = json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} });
    let res_inited = dispatch_client(inited, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res_inited.unwrap());

    // 3. Shutdown
    let shut = json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" });
    let res_shut = dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(res_shut.unwrap());

    // 4. Exit
    let exit_msg = json!({ "jsonrpc": "2.0", "method": "exit" });
    let res_exit = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(!res_exit.unwrap());
}

#[test]
fn test_exit_does_not_mutate_other_state() {
    let mut lc = LifecycleManager::new();
    let init = json!({ "params": { "capabilities": {} } });
    lc.handle_initialize(&init).unwrap();
    lc.handle_shutdown().unwrap();

    assert!(lc.is_shutdown());
    let _ = lc.handle_exit();
    assert!(lc.is_shutdown());
}

#[test]
fn test_exit_with_pending_cancelled_requests() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let cancel1 = json!({ "jsonrpc": "2.0", "method": "$/cancelRequest", "params": { "id": 42 } });
    dispatch_client(cancel1, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    let cancel2 = json!({ "jsonrpc": "2.0", "method": "$/cancelRequest", "params": { "id": "req-99" } });
    dispatch_client(cancel2, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } });
    dispatch_client(init, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    let shut = json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" });
    dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    let exit_msg = json!({ "jsonrpc": "2.0", "method": "exit" });
    let res = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(!res.unwrap());
    assert_eq!(*can.lock().unwrap(), HashSet::from([json!(42), json!("req-99")]));
}

#[test]
fn test_exit_returns_exit_code_zero_signal() {
    let mut lc = LifecycleManager::new();
    let init = json!({ "params": { "capabilities": {} } });
    lc.handle_initialize(&init).unwrap();
    lc.handle_shutdown().unwrap();
    let res = lc.handle_exit();
    assert_eq!(res.unwrap(), ());
}

#[test]
fn test_exit_error_message_exact() {
    let lc = LifecycleManager::new();
    let err = lc.handle_exit().unwrap_err();
    assert_eq!(err.to_string(), "exit notification received before shutdown");
}

#[test]
fn test_exit_with_numeric_id_handled() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } });
    dispatch_client(init, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    let shut = json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" });
    dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    let exit_msg = json!({ "jsonrpc": "2.0", "id": 99, "method": "exit" });
    let res = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(!res.unwrap());
}

#[test]
fn test_exit_notification_no_response_written() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } });
    dispatch_client(init, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    let shut = json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" });
    dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    out.clear();
    let exit_msg = json!({ "jsonrpc": "2.0", "method": "exit" });
    dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    assert_eq!(out, b"");
}

#[test]
fn test_exit_cleans_up_no_deadlock() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    ov.open_document("file:///tmp/active_overlay.rs", "/tmp/active_overlay.rs", 1, "fn main() {}\n".to_string());
    let init = json!({ "jsonrpc": "2.0", "id": 100, "method": "initialize", "params": { "capabilities": {} } });
    dispatch_client(init, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    let shut = json!({ "jsonrpc": "2.0", "id": 200, "method": "shutdown" });
    dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();

    let exit_msg = json!({ "jsonrpc": "2.0", "method": "exit" });
    let res = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(!res.unwrap());
    assert_eq!(ov.get("/tmp/active_overlay.rs").unwrap().version, 1);
    assert!(lc.is_shutdown());
}

#[test]
fn test_exit_preserves_uninitialized_error_distinction() {
    let lc = LifecycleManager::new();
    assert!(!lc.is_initialized());
    assert!(!lc.is_shutdown());
    let err = lc.handle_exit().unwrap_err();
    assert_eq!(err.to_string(), "exit notification received before shutdown");
}
