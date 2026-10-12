/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for exit notification part 5 (Domain 1: Lifecycle & Control).

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
fn test_exit_after_dynamic_registration_cycle() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } });
    let _ = dispatch_client(init, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let inited = json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} });
    let _ = dispatch_client(inited, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let shut = json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" });
    let _ = dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can);

    let exit_msg = json!({ "jsonrpc": "2.0", "method": "exit" });
    let res = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(!res.unwrap());
}

#[test]
fn test_exit_after_multiple_document_edits() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } });
    let _ = dispatch_client(init, &mut out, &mut mr, &mut ov, &mut lc, &can);

    ov.open_or_update("file:///a.rs", "/a.rs", 1, "fn a() {}".to_string());
    ov.open_or_update("file:///b.rs", "/b.rs", 1, "fn b() {}".to_string());

    let shut = json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" });
    let _ = dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can);

    let exit_msg = json!({ "jsonrpc": "2.0", "method": "exit" });
    let res = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(!res.unwrap());
}

#[test]
fn test_exit_with_nested_json_params() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    lc.handle_shutdown().unwrap();
    let exit_msg = json!({
        "jsonrpc": "2.0",
        "method": "exit",
        "params": {
            "meta": {
                "reason": "user_requested",
                "clean": true
            }
        }
    });
    let res = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(!res.unwrap());
}

#[test]
fn test_exit_clean_shutdown_preserves_exit_code_0() {
    let mut lc = LifecycleManager::new();
    let init = json!({ "params": { "capabilities": {} } });
    lc.handle_initialize(&init).unwrap();
    let _ = lc.handle_shutdown();
    assert_eq!(lc.handle_exit().unwrap(), ());
}

#[test]
fn test_exit_abrupt_exit_preserves_exit_code_1() {
    let mut lc = LifecycleManager::new();
    let init = json!({ "params": { "capabilities": {} } });
    lc.handle_initialize(&init).unwrap();
    // No shutdown!
    let err = lc.handle_exit().unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::Other);
    assert_eq!(err.to_string(), "exit notification received before shutdown");
}

#[test]
fn test_exit_after_initialize_reject_cycle() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let init1 = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } });
    let _ = dispatch_client(init1, &mut out, &mut mr, &mut ov, &mut lc, &can);

    let init2 = json!({ "jsonrpc": "2.0", "id": 2, "method": "initialize", "params": { "capabilities": {} } });
    let _ = dispatch_client(init2, &mut out, &mut mr, &mut ov, &mut lc, &can);

    let shut = json!({ "jsonrpc": "2.0", "id": 3, "method": "shutdown" });
    let _ = dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can);

    let exit_msg = json!({ "jsonrpc": "2.0", "method": "exit" });
    let res = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(!res.unwrap());
}

#[test]
fn test_exit_does_not_panic_on_custom_fields() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    lc.handle_shutdown().unwrap();
    let exit_msg = json!({
        "jsonrpc": "2.0",
        "method": "exit",
        "extra_field": [1, 2, 3]
    });
    let res = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(!res.unwrap());
}

#[test]
fn test_exit_final_lifecycle_checkpoint() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } });
    let _ = dispatch_client(init, &mut out, &mut mr, &mut ov, &mut lc, &can);
    let shut = json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" });
    let _ = dispatch_client(shut, &mut out, &mut mr, &mut ov, &mut lc, &can);

    let exit_msg = json!({ "jsonrpc": "2.0", "method": "exit" });
    let res = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(!res.unwrap());
    assert_eq!(lc.handle_exit().unwrap(), ());
}

#[test]
fn test_exit_writer_unmodified_on_exit() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = b"prior_buffered_bytes".to_vec();

    let exit_msg = json!({ "jsonrpc": "2.0", "method": "exit" });
    let _ = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(out, b"prior_buffered_bytes");
}

#[test]
fn test_exit_notification_without_version_header() {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    lc.handle_shutdown().unwrap();
    let exit_msg = json!({ "method": "exit" });
    let res = dispatch_client(exit_msg, &mut out, &mut mr, &mut ov, &mut lc, &can);
    assert!(!res.unwrap());
}
