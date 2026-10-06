/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::common::*;
use crate::lsp::*;
use serde_json::{json, Value};
use std::io::Cursor;

#[test]
fn test_position_converter_caching_and_utf16() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_conv_test_{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let file_path = temp_dir.join("test.rs");

    let content = "fn main() {\n    let s = \"привет\";\n    let rocket = \"🚀!\";\n}\n";
    std::fs::write(&file_path, content).unwrap();

    let path_str = file_path.to_str().unwrap();
    let mut conv = PositionConverter::new();

    let utf16_col = conv.char_col_to_utf16_col(path_str, 2, 14);
    assert_eq!(utf16_col, 13);
    assert!(conv.cache.contains_key(path_str));

    let utf16_col_bang = conv.char_col_to_utf16_col(path_str, 3, 20);
    assert_eq!(utf16_col_bang, 20);

    let char_col = conv.utf16_col_to_char_col(path_str, 3, 20);
    assert_eq!(char_col, 20);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_lsp_server_stream_initialize_and_capabilities() {
    let mut input = Vec::new();
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "workspaceFolders": [
                { "uri": "file:///tmp/workspace", "name": "workspace" }
            ],
            "capabilities": {}
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {}
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "workspace/didRenameFiles",
        "params": {
            "files": [
                { "oldUri": "file:///tmp/old.rs", "newUri": "file:///tmp/new.rs" }
            ]
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "workspace/didDeleteFiles",
        "params": {
            "files": [
                { "uri": "file:///tmp/new.rs" }
            ]
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": "file:///tmp/unopened.rs", "version": 2 },
            "contentChanges": [
                { "text": "fn fallback() {}\n" }
            ]
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "shutdown",
        "params": null
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "exit"
    })));

    let mut output = Vec::new();
    let res = run_lsp_server_stream(Cursor::new(input), &mut output, None);
    assert!(res.is_ok());

    let responses = parse_lsp_responses(&output);
    assert_eq!(responses.len(), 2);

    let init_resp = &responses[0];
    assert_eq!(init_resp["id"], 1);
    let caps = &init_resp["result"]["capabilities"];
    assert_eq!(caps["textDocumentSync"]["change"], 2);
    assert_eq!(caps["workspace"]["workspaceFolders"]["supported"], true);

    let shut_resp = &responses[1];
    assert_eq!(shut_resp["id"], 2);
    assert_eq!(shut_resp["result"], Value::Null);
}

#[test]
fn test_lsp_server_stream_rejects_requests_before_initialize() {
    let mut input = Vec::new();
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 42,
        "method": "textDocument/documentSymbol",
        "params": { "textDocument": { "uri": "file:///tmp/test.rs" } }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {} }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "shutdown",
        "params": null
    })));
    // Post-shutdown request (lines 157-169)
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 99,
        "method": "textDocument/documentSymbol",
        "params": { "textDocument": { "uri": "file:///tmp/test.rs" } }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "exit"
    })));

    let mut output = Vec::new();
    let res = run_lsp_server_stream(Cursor::new(input), &mut output, None);
    assert!(res.is_ok());

    let responses = parse_lsp_responses(&output);
    assert_eq!(responses.len(), 4);

    // Pre-initialize request MUST be rejected with ServerNotInitialized (-32002)
    assert_eq!(responses[0]["id"], 42);
    assert_eq!(responses[0]["error"]["code"], -32002);

    // initialize succeeds
    assert_eq!(responses[1]["id"], 1);
    assert!(responses[1]["result"]["capabilities"].is_object());

    // shutdown succeeds
    assert_eq!(responses[2]["id"], 2);

    // Post-shutdown request MUST be rejected with Server is shutting down (-32600)
    assert_eq!(responses[3]["id"], 99);
    assert_eq!(responses[3]["error"]["code"], -32600);
}

#[test]
fn test_lsp_server_stream_request_cancellation() {
    let mut input = Vec::new();
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {} }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "$/cancelRequest",
        "params": { "id": 100 }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 100,
        "method": "textDocument/definition",
        "params": {
            "textDocument": { "uri": "file:///tmp/dummy.rs" },
            "position": { "line": 0, "character": 0 }
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "shutdown",
        "params": null
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "exit"
    })));

    let mut output = Vec::new();
    let res = run_lsp_server_stream(Cursor::new(input), &mut output, None);
    assert!(res.is_ok());

    let responses = parse_lsp_responses(&output);
    assert_eq!(responses.len(), 3);

    // Verify request 100 received cancellation error -32800
    let cancel_resp = &responses[1];
    assert_eq!(cancel_resp["id"], 100);
    assert_eq!(cancel_resp["error"]["code"], -32800);
}

#[test]
fn test_lsp_server_stream_exit_before_shutdown_returns_err() {
    let mut input = Vec::new();
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "exit"
    })));

    let mut output = Vec::new();
    let res = run_lsp_server_stream(Cursor::new(input), &mut output, None);
    assert!(res.is_err());
}

#[test]
fn test_lsp_server_stream_shutdown_rejects_further_requests() {
    let mut input = Vec::new();
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {} }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "shutdown",
        "params": null
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": "file:///tmp/dummy.rs" },
            "position": { "line": 0, "character": 0 }
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "exit"
    })));

    let mut output = Vec::new();
    let res = run_lsp_server_stream(Cursor::new(input), &mut output, None);
    assert!(res.is_ok());

    let responses = parse_lsp_responses(&output);
    assert_eq!(responses.len(), 3);
    assert_eq!(responses[2]["id"], 3);
    assert_eq!(responses[2]["error"]["code"], -32600);
}

#[test]
fn test_lifecycle_default_and_double_shutdown() {
    let mut lm = LifecycleManager::default();
    assert!(!lm.is_initialized());
    assert!(!lm.is_shutdown());
    let _ = lm.handle_initialize(&json!({ "capabilities": {} }));
    let s1 = lm.handle_shutdown();
    assert!(s1.is_ok());
    assert!(lm.is_shutdown());
    let s2 = lm.handle_shutdown();
    assert!(s2.is_err());
    assert_eq!(s2.unwrap_err().0, -32600);
}
