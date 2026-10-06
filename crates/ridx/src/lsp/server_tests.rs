/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::*;
use serde_json::json;
use std::io::Cursor;

fn format_lsp_msg(val: &serde_json::Value) -> String {
    let body = serde_json::to_string(val).unwrap();
    format!("Content-Length: {}\r\n\r\n{}", body.len(), body)
}

#[test]
fn test_lsp_server_stream_lifecycle_and_methods() {
    let mut input = String::new();

    // 1. Request before initialize
    input.push_str(&format_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "textDocument/definition",
        "params": {}
    })));

    // 2. Malformed / missing Content-Length line
    input.push_str("Invalid-Header: 123\r\n\r\n");

    // 3. Initialize request with folders when multi_root is empty
    input.push_str(&format_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "initialize",
        "params": {
            "workspaceFolders": [{ "uri": "file:///tmp/mock_folder", "name": "mock" }]
        }
    })));

    // 4. textDocument/didChange on an unopened file
    input.push_str(&format_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": "file:///tmp/unopened_file.rs", "version": 1 },
            "contentChanges": [{ "text": "fn new_content() {}\n" }]
        }
    })));

    // 5. workspace/didChangeWorkspaceFolders
    input.push_str(&format_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWorkspaceFolders",
        "params": {
            "event": {
                "added": [{ "uri": "file:///tmp/added", "name": "added" }],
                "removed": []
            }
        }
    })));

    // 6. Request cancellation
    input.push_str(&format_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "$/cancelRequest",
        "params": { "id": 999 }
    })));

    // 7. Cancelled request execution
    input.push_str(&format_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 999,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": "file:///tmp/unopened_file.rs" },
            "position": { "line": 0, "character": 0 }
        }
    })));

    // 8. Exercise dispatch handlers
    let methods = [
        "textDocument/definition",
        "textDocument/declaration",
        "textDocument/typeDefinition",
        "textDocument/implementation",
        "textDocument/references",
        "textDocument/documentHighlight",
        "textDocument/documentSymbol",
        "workspace/symbol",
        "workspaceSymbol/resolve",
        "textDocument/hover",
        "textDocument/completion",
        "completionItem/resolve",
        "textDocument/signatureHelp",
        "textDocument/formatting",
        "textDocument/prepareRename",
        "textDocument/rename",
        "textDocument/prepareCallHierarchy",
        "callHierarchy/incomingCalls",
        "callHierarchy/outgoingCalls",
        "textDocument/prepareTypeHierarchy",
        "typeHierarchy/supertypes",
        "typeHierarchy/subtypes",
        "textDocument/diagnostic",
        "textDocument/selectionRange",
        "textDocument/documentLink",
        "documentLink/resolve",
        "textDocument/semanticTokens/full",
        "textDocument/foldingRange",
        "textDocument/codeAction",
        "codeAction/resolve",
        "textDocument/codeLens",
        "codeLens/resolve",
        "textDocument/inlayHint",
        "textDocument/moniker",
        "workspace/executeCommand",
        "textDocument/documentColor",
        "textDocument/colorPresentation",
        "textDocument/linkedEditingRange",
        "textDocument/inlineValue",
        "textDocument/inlineCompletion",
        "textDocument/content",
    ];

    let mut req_id = 100;
    for m in methods {
        req_id += 1;
        input.push_str(&format_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": req_id,
            "method": m,
            "params": {
                "textDocument": { "uri": "file:///tmp/unopened_file.rs" },
                "position": { "line": 0, "character": 0 },
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
                "item": { "name": "test", "uri": "file:///tmp/unopened_file.rs", "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } } },
                "newName": "new_name",
                "command": "test.cmd"
            }
        })));
    }

    // 9. shutdown
    input.push_str(&format_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "shutdown"
    })));

    // 10. Request after shutdown
    input.push_str(&format_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "textDocument/definition",
        "params": {}
    })));

    // 11. exit
    input.push_str(&format_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "exit"
    })));

    let reader = Cursor::new(input.into_bytes());
    let mut output = Vec::new();
    let res = run_lsp_server_stream(reader, &mut output, None);
    assert!(res.is_ok());
    let out_str = String::from_utf8_lossy(&output);
    assert!(out_str.contains("Server not initialized"));
    assert!(out_str.contains("Server is shutting down"));
}
