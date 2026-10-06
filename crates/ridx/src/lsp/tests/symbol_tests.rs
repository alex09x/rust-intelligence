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
fn test_lsp_server_stream_workspace_symbol_resolve() {
    let mut input = Vec::new();
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {} }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 10,
        "method": "workspaceSymbol/resolve",
        "params": {
            "name": "my_symbol",
            "kind": 12
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
    assert_eq!(responses[1]["id"], 10);
    assert_eq!(responses[1]["result"]["name"], "my_symbol");
}

#[test]
fn test_lsp_server_stream_incremental_sync_and_symbols() {
    let mut input = Vec::new();
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {} }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": "file:///tmp/app.rs",
                "version": 1,
                "text": "fn compute() -> i32 {\n    let val = 100;\n    val\n}\n"
            }
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "textDocument/documentSymbol",
        "params": {
            "textDocument": { "uri": "file:///tmp/app.rs" }
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 3,
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
    assert_eq!(responses[1]["id"], 2);
    let syms = responses[1]["result"].as_array().unwrap();
    assert!(!syms.is_empty());
}

#[test]
fn test_lsp_server_stream_file_operations_rename_and_delete() {
    let mut input = Vec::new();
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {} }
    })));
    let old_uri = "file:///tmp/old_file.rs";
    let new_uri = "file:///tmp/new_file.rs";
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": old_uri,
                "languageId": "rust",
                "version": 1,
                "text": "fn old_func() {}\n"
            }
        }
    })));
    // Rename old_file to new_file
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "workspace/didRenameFiles",
        "params": {
            "files": [
                { "oldUri": old_uri, "newUri": new_uri }
            ]
        }
    })));
    // Query symbols on new_file
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "textDocument/documentSymbol",
        "params": {
            "textDocument": { "uri": new_uri }
        }
    })));
    // Delete new_file
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "workspace/didDeleteFiles",
        "params": {
            "files": [
                { "uri": new_uri }
            ]
        }
    })));
    // Query symbols on new_file again
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "textDocument/documentSymbol",
        "params": {
            "textDocument": { "uri": new_uri }
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 4,
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
    assert_eq!(responses.len(), 4);

    let sym_resp = &responses[1];
    assert_eq!(sym_resp["id"], 2);
    let symbols = sym_resp["result"].as_array().expect("array of symbols");
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0]["name"], "old_func");

    let sym_resp_del = &responses[2];
    assert_eq!(sym_resp_del["id"], 3);
    let symbols_del = sym_resp_del["result"].as_array().expect("empty array");
    assert!(symbols_del.is_empty());
}

#[test]
fn test_lsp_server_stream_work_done_progress_create() {
    let mut input = Vec::new();
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {} }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 20,
        "method": "window/workDoneProgress/create",
        "params": { "token": "token-xyz" }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 21,
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

    let prog_resp = &responses[1];
    assert_eq!(prog_resp["id"], 20);
    assert_eq!(prog_resp["result"], Value::Null);
}
