/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/prepareRename request (Domain 8: Editing & Workspace Edits).

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
        "id": 80,
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
fn test_prepare_rename_local_let_binding_range_and_placeholder() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/prep_rename.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let counter = 10;\n    counter + 1;\n}\n");

    let val = send_prepare_rename_req(uri, 1, 9, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["id"], 80);
    let res = &val["result"];
    assert_eq!(res["placeholder"], "counter");
    assert_eq!(res["range"]["start"]["line"], 1);
    assert_eq!(res["range"]["start"]["character"], 8);
    assert_eq!(res["range"]["end"]["character"], 15);
}

#[test]
fn test_prepare_rename_multiple_bindings_in_scope() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/prep_multi.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let foo = 1;\n    let bar = 2;\n}\n");

    let val_foo = send_prepare_rename_req(uri, 1, 9, &mut mr, &mut ov, &mut lc, &can);
    let val_bar = send_prepare_rename_req(uri, 2, 9, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val_foo["result"]["placeholder"], "foo");
    assert_eq!(val_bar["result"]["placeholder"], "bar");
}

#[test]
fn test_prepare_rename_fn_parameter_binding() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/prep_param.rs";
    open_test_file(&mut ov, uri, 1, "fn compute(alpha: i32) -> i32 {\n    alpha + 1\n}\n");

    let val = send_prepare_rename_req(uri, 0, 12, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["result"]["placeholder"], "alpha");
}

#[test]
fn test_prepare_rename_whitespace_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/prep_ws.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let x = 1;\n}\n");

    let val = send_prepare_rename_req(uri, 0, 0, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["error"]["code"], -32803);
    assert_eq!(val["error"]["message"], "Cannot prepare rename: no identifier found at this position");
}

#[test]
fn test_prepare_rename_comment_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/prep_comment.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    // some comment\n}\n");

    let val = send_prepare_rename_req(uri, 1, 8, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["error"]["code"], -32803);
    assert_eq!(val["error"]["message"], "Cannot prepare rename: no identifier found at this position");
}

#[test]
fn test_prepare_rename_keyword_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/prep_kw.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let x = 1;\n}\n");

    // Position on "let" keyword
    let val = send_prepare_rename_req(uri, 1, 4, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["error"]["code"], -32803);
    assert_eq!(val["error"]["message"], "Cannot prepare rename: no identifier found at this position");
}

#[test]
fn test_prepare_rename_empty_file_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/prep_empty.rs";
    open_test_file(&mut ov, uri, 1, "");

    let val = send_prepare_rename_req(uri, 0, 0, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["error"]["code"], -32803);
    assert_eq!(val["error"]["message"], "Cannot prepare rename: no identifier found at this position");
}

#[test]
fn test_prepare_rename_out_of_bounds_position_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/prep_oob.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {}\n");

    let val = send_prepare_rename_req(uri, 99, 99, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["error"]["code"], -32803);
    assert_eq!(val["error"]["message"], "Cannot prepare rename: no identifier found at this position");
}

#[test]
fn test_prepare_rename_punctuation_symbol_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/prep_punct.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let x = 1 + 2;\n}\n");

    // Position on "+" operator
    let val = send_prepare_rename_req(uri, 1, 14, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val["error"]["code"], -32803);
    assert_eq!(val["error"]["message"], "Cannot prepare rename: no identifier found at this position");
}

#[test]
fn test_prepare_rename_preserves_request_id() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/prep_id.rs";
    open_test_file(&mut ov, uri, 1, "fn main() {\n    let my_id = 42;\n}\n");

    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": "rename-prep-req-999",
        "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": 1, "character": 9 }
        }
    });
    dispatch_lsp_message(LspMessage::Client(req), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
    let val = extract_response(&out);
    assert_eq!(val["id"], "rename-prep-req-999");
}
