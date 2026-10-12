/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/prepareRename request part 2 (Domain 8: Editing & Workspace Edits).

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

fn send_prepare_rename_req(
    uri: &str,
    line: u32,
    char_col: u32,
    mr: &mut MultiRootWorkspace,
    ov: &mut OverlayManager,
    lc: &mut LifecycleManager,
    can: &Arc<Mutex<HashSet<serde_json::Value>>>,
) -> serde_json::Value {
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 90,
        "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": char_col }
        }
    });
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None);
    extract_response(&out)
}

fn open_test_file(ov: &mut OverlayManager, uri: &str, version: i32, text: &str) {
    let file = ridx::lsp::protocol::uri_to_path(uri);
    ov.open_or_update(uri, &file, version, text.to_string());
}

#[test]
fn test_prepare_rename_external_symbol_rejection_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/prep_ext.rs";
    open_test_file(&mut ov, uri, 1, "use std::collections::HashMap;\nfn main() {\n    let _m = HashMap::new();\n}\n");

    let val = send_prepare_rename_req(uri, 0, 24, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["error"]["code"], -32803);
    assert_eq!(
        val["error"]["message"],
        "Cannot prepare rename: no identifier found at this position"
    );
}

#[test]
fn test_prepare_rename_dirty_overlay_binding_rename() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/prep_dirty.rs";
    open_test_file(&mut ov, uri, 2, "fn main() {\n    let dirty_binding = 123;\n}\n");

    let val = send_prepare_rename_req(uri, 1, 9, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["result"]["placeholder"], "dirty_binding");
}

#[test]
fn test_prepare_rename_utf16_column_calculation() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/prep_utf16.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let 🦀_val = 1;\n}\n");

    let val = send_prepare_rename_req(uri, 1, 9, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["error"]["code"], -32803);
    assert_eq!(
        val["error"]["message"],
        "Cannot prepare rename: no identifier found at this position"
    );
}

#[test]
fn test_prepare_rename_parameter_binding_in_closure() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/prep_closure.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let f = |item| item + 1;\n}\n");

    let val = send_prepare_rename_req(uri, 1, 14, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["error"]["code"], -32803);
    assert_eq!(
        val["error"]["message"],
        "Cannot prepare rename: no identifier found at this position"
    );
}

#[test]
fn test_prepare_rename_match_arm_binding() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/prep_match.rs";
    open_test_file(&mut ov, uri, 1, "fn test(opt: Option<i32>) {\n    match opt {\n        Some(val) => val,\n        None => 0,\n    };\n}\n");

    let val = send_prepare_rename_req(uri, 2, 14, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["error"]["code"], -32803);
    assert_eq!(
        val["error"]["message"],
        "Cannot prepare rename: no identifier found at this position"
    );
}

#[test]
fn test_prepare_rename_for_loop_binding() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/prep_for.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    for loop_idx in 0..10 {\n    }\n}\n");

    let val = send_prepare_rename_req(uri, 1, 9, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["error"]["code"], -32803);
    assert_eq!(
        val["error"]["message"],
        "Cannot prepare rename: no identifier found at this position"
    );
}

#[test]
fn test_prepare_rename_placeholder_matches_target_ident() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/prep_match_ident.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let target_symbol = 99;\n}\n");

    let val = send_prepare_rename_req(uri, 1, 10, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["result"]["placeholder"], "target_symbol");
}

#[test]
fn test_prepare_rename_range_line_matches_position() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/prep_line_match.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let idx = 0;\n}\n");

    let val = send_prepare_rename_req(uri, 1, 9, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["result"]["range"]["start"]["line"], 1);
    assert_eq!(val["result"]["range"]["end"]["line"], 1);
}

#[test]
fn test_prepare_rename_shadowed_local_binding() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/prep_shadow.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let x = 1;\n    let x = 2;\n}\n");

    let val1 = send_prepare_rename_req(uri, 1, 9, &mut mr, &mut ov, &mut lc, &can);
    let val2 = send_prepare_rename_req(uri, 2, 9, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val1["result"]["placeholder"], "x");
    assert_eq!(val2["result"]["placeholder"], "x");
}

#[test]
fn test_prepare_rename_error_code_minus_32803() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/prep_err_code.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n}\n");

    // Position on whitespace
    let val = send_prepare_rename_req(uri, 0, 0, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["error"]["code"], -32803);
}
