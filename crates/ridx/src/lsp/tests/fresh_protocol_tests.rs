/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `ridx/status` and `minVersion` through a whole session, against the lifecycle rules every request follows.

use super::common::*;
use crate::lsp::*;
use serde_json::json;
use std::io::Cursor;

/// Review of a6909dbe: status answered before `initialize`, after `shutdown` and to a notification; a request held for
/// `minVersion` outlived `shutdown`.
#[test]
fn test_status_and_held_requests_follow_the_lifecycle() {
    let folder = std::env::temp_dir().join(format!("ridx_fresh_protocol_{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let uri = format!("file://{}/lib.rs", folder.display());
    let mut input = Vec::new();
    let mut send = |v: serde_json::Value| input.extend(make_lsp_msg(&v));
    send(json!({ "jsonrpc": "2.0", "id": 1, "method": "ridx/status", "params": {} }));
    send(
        json!({ "jsonrpc": "2.0", "id": 2, "method": "initialize", "params": { "workspaceFolders": [{ "uri": format!("file://{}", folder.display()), "name": "ws" }], "capabilities": {} } }),
    );
    send(json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }));
    send(json!({ "jsonrpc": "2.0", "method": "ridx/status", "params": {} }));
    send(json!({ "jsonrpc": "2.0", "id": 3, "method": "ridx/status", "params": {} }));
    send(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": { "uri": uri, "languageId": "rust", "version": 1, "text": "pub fn a() {}\n" } } }),
    );
    // No index holds the file: version 1 is never covered, the request waits.
    send(json!({ "jsonrpc": "2.0", "id": 4, "method": "textDocument/documentSymbol", "params": { "textDocument": { "uri": uri }, "minVersion": 1 } }));
    send(json!({ "jsonrpc": "2.0", "id": 99, "method": "shutdown", "params": null }));
    send(json!({ "jsonrpc": "2.0", "id": 5, "method": "ridx/status", "params": {} }));
    send(json!({ "jsonrpc": "2.0", "method": "exit" }));

    let mut output = Vec::new();
    assert!(run_lsp_server_stream(Cursor::new(input), &mut output, None).is_ok());
    let responses = parse_lsp_responses(&output);
    let by_id = |id: i64| responses.iter().find(|r| r["id"] == id).unwrap_or_else(|| panic!("no answer {id}: {responses:?}"));
    assert_eq!(by_id(1)["error"]["code"], -32002, "status before initialize");
    assert!(by_id(3)["result"]["roots"].is_array(), "status after initialize: {}", by_id(3));
    assert!(!responses.iter().any(|r| r["id"].is_null() && r.get("result").is_some()), "a notification gets no answer");
    assert_eq!(by_id(4)["error"]["code"], -32600, "a held request ends with the shutdown");
    assert_eq!(by_id(5)["error"]["code"], -32600, "status after shutdown");
    let _ = std::fs::remove_dir_all(&folder);
}
