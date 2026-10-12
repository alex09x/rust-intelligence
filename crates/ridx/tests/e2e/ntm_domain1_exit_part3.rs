/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for exit notification part 3 (Domain 1: Lifecycle & Control).

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
fn test_exit_without_shutdown_returns_false_signal() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } });
    dispatch_client(init, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    let initialized = json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} });
    dispatch_client(initialized, &mut out, &mut mr, &mut ov, &mut lc, &can).unwrap();
    out.clear();

    let exit_msg = json!({ "jsonrpc": "2.0", "method": "exit" });
    let res = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let err = res.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::Other);
    assert_eq!(err.to_string(), "exit notification received before shutdown");
    assert_eq!(out, b"");
}

#[test]
fn test_exit_after_shutdown_returns_false_signal() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } });
    let _ = dispatch_client(init, &mut out, &mut mr, &mut ov, &mut lc, &can);
    out.clear();

    let shut = json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" });
    let _ = dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can);
    out.clear();

    let exit_msg = json!({ "jsonrpc": "2.0", "method": "exit" });
    let res = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(!res.unwrap());
}

#[test]
fn test_exit_produces_no_stdout_output() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let exit_msg = json!({ "jsonrpc": "2.0", "method": "exit" });
    let _ = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(out, b"");
}

#[test]
fn test_exit_exit_code_0_after_clean_shutdown() {
    let mut lc = LifecycleManager::new();
    let init = json!({ "params": { "capabilities": {} } });
    lc.handle_initialize(&init).unwrap();
    lc.handle_shutdown().unwrap();

    assert_eq!(lc.handle_exit().unwrap(), ());
}

#[test]
fn test_exit_exit_code_1_without_shutdown() {
    let lc = LifecycleManager::new();
    let err = lc.handle_exit().unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::Other);
    assert_eq!(err.to_string(), "exit notification received before shutdown");
}

#[test]
fn test_exit_exit_code_1_after_initialize_without_shutdown() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } });
    let _ = dispatch_client(init, &mut out, &mut mr, &mut ov, &mut lc, &can);
    out.clear();

    let exit_msg = json!({ "jsonrpc": "2.0", "method": "exit" });
    let res = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let err = res.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::Other);
    assert_eq!(err.to_string(), "exit notification received before shutdown");
}

#[test]
fn test_exit_with_null_params_safe() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    lc.handle_shutdown().unwrap();
    let exit_msg = json!({ "jsonrpc": "2.0", "method": "exit", "params": null });
    let res = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(!res.unwrap());
}

#[test]
fn test_exit_with_empty_params_safe() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    lc.handle_shutdown().unwrap();
    let exit_msg = json!({ "jsonrpc": "2.0", "method": "exit", "params": {} });
    let res = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(!res.unwrap());
}

#[test]
fn test_exit_without_params_field_safe() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    lc.handle_shutdown().unwrap();
    let exit_msg = json!({ "jsonrpc": "2.0", "method": "exit" });
    let res = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(!res.unwrap());
}

#[test]
fn test_exit_does_not_mutate_workspace_roots() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let exit_msg = json!({ "jsonrpc": "2.0", "method": "exit" });
    let _ = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(mr.len(), 0);
}
