/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! End-to-end integration tests for LSP streaming, capabilities, lifecycle, and concurrency.

use super::*;
use ridx_format::writer::{IndexWriter, RawDefinition, RawReference, RawType};
use ridx_format::DefKind;
use serde_json::json;
use std::io::Cursor;

pub(crate) fn make_lsp_msg(val: &Value) -> Vec<u8> {
    let s = val.to_string();
    format!("Content-Length: {}\r\n\r\n{}", s.len(), s).into_bytes()
}

pub(crate) fn parse_lsp_responses(bytes: &[u8]) -> Vec<Value> {
    let mut results = Vec::new();
    let mut cur = 0;
    while cur < bytes.len() {
        let slice = &bytes[cur..];
        let Some(header_end) = slice.windows(4).position(|w| w == b"\r\n\r\n") else {
            break;
        };
        let header_str = String::from_utf8_lossy(&slice[..header_end]);
        let mut content_len = 0;
        for line in header_str.lines() {
            if let Some(rest) = line.strip_prefix("Content-Length:") {
                content_len = rest.trim().parse::<usize>().unwrap_or(0);
            }
        }
        let body_start = cur + header_end + 4;
        let body_end = body_start + content_len;
        if body_end > bytes.len() {
            break;
        }
        if let Ok(val) = serde_json::from_slice::<Value>(&bytes[body_start..body_end]) {
            results.push(val);
        }
        cur = body_end;
    }
    results
}

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

#[test]
fn test_lsp_server_stream_stage2_declaration_and_highlight() {
    let mut input = Vec::new();
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {} }
    })));
    let doc_uri = "file:///tmp/nav_test.rs";
    let code = "fn compute() {\n    let val = 42;\n    let double = val * 2;\n}\n";
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": doc_uri,
                "languageId": "rust",
                "version": 1,
                "text": code
            }
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 30,
        "method": "textDocument/declaration",
        "params": {
            "textDocument": { "uri": doc_uri },
            "position": { "line": 2, "character": 18 }
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 31,
        "method": "textDocument/documentHighlight",
        "params": {
            "textDocument": { "uri": doc_uri },
            "position": { "line": 1, "character": 8 }
        }
    })));
    let sym_to_resolve = json!({
        "name": "compute",
        "kind": 12,
        "location": {
            "uri": doc_uri,
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 0, "character": 12 }
            }
        }
    });
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 32,
        "method": "workspace/symbol/resolve",
        "params": sym_to_resolve
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 33,
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
    assert_eq!(responses.len(), 5);

    let decl_resp = &responses[1];
    assert_eq!(decl_resp["id"], 30);
    assert_eq!(decl_resp["result"]["range"]["start"]["line"], 1);

    let hl_resp = &responses[2];
    assert_eq!(hl_resp["id"], 31);
    let hls = hl_resp["result"].as_array().expect("array of highlights");
    assert!(!hls.is_empty());
    assert_eq!(hls[0]["range"]["start"]["line"], 1);

    let res_resp = &responses[3];
    assert_eq!(res_resp["id"], 32);
    assert_eq!(res_resp["result"]["name"], "compute");
}

#[test]
fn test_lsp_server_stream_stage3_completion_and_diagnostic() {
    let mut input = Vec::new();
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {} }
    })));
    let doc_uri = "file:///tmp/completion_test.rs";
    let code = "fn helper() {\n    let candidate_var = 123;\n}\n";
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": doc_uri,
                "languageId": "rust",
                "version": 1,
                "text": code
            }
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 40,
        "method": "textDocument/completion",
        "params": {
            "textDocument": { "uri": doc_uri },
            "position": { "line": 1, "character": 10 }
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 41,
        "method": "textDocument/diagnostic",
        "params": {
            "textDocument": { "uri": doc_uri }
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 42,
        "method": "textDocument/formatting",
        "params": {
            "textDocument": { "uri": doc_uri },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 43,
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
    assert_eq!(responses.len(), 5);

    let comp_resp = &responses[1];
    assert_eq!(comp_resp["id"], 40);
    let items = comp_resp["result"]["items"]
        .as_array()
        .expect("items array");
    assert!(items.iter().any(|i| i["label"] == "candidate_var"));

    let diag_resp = &responses[2];
    assert_eq!(diag_resp["id"], 41);
    assert_eq!(diag_resp["result"]["kind"], "full");

    let fmt_resp = &responses[3];
    assert_eq!(fmt_resp["id"], 42);
    assert!(fmt_resp["result"].is_array());
}

#[test]
fn test_lsp_server_stream_stage4_structure_and_presentation() {
    let mut input = Vec::new();
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {} }
    })));
    let doc_uri = "file:///tmp/stage4_test.rs";
    let code = "fn main() {\n    let val_x = 10;\n    let val_y = 20;\n}\n";
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": doc_uri,
                "languageId": "rust",
                "version": 1,
                "text": code
            }
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 50,
        "method": "textDocument/foldingRange",
        "params": {
            "textDocument": { "uri": doc_uri }
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 51,
        "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": doc_uri },
            "positions": [{ "line": 1, "character": 8 }]
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 52,
        "method": "textDocument/semanticTokens/full",
        "params": {
            "textDocument": { "uri": doc_uri }
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 53,
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
    assert_eq!(responses.len(), 5);

    let fold_resp = &responses[1];
    assert_eq!(fold_resp["id"], 50);
    let fold_ranges = fold_resp["result"].as_array().expect("folding array");
    assert!(!fold_ranges.is_empty());

    let sel_resp = &responses[2];
    assert_eq!(sel_resp["id"], 51);
    let sel_ranges = sel_resp["result"].as_array().expect("selection array");
    assert!(!sel_ranges.is_empty());

    let sem_resp = &responses[3];
    assert_eq!(sem_resp["id"], 52);
    let sem_data = sem_resp["result"]["data"].as_array().expect("data array");
    assert!(!sem_data.is_empty());
}

#[test]
fn test_lsp_server_stream_stage5_safe_code_changes() {
    let mut input = Vec::new();
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {} }
    })));
    let doc_uri = "file:///tmp/stage5_test.rs";
    let code = "fn main() {\n    let mut counter = 0;\n    counter += 1;\n}\n";
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": doc_uri,
                "languageId": "rust",
                "version": 1,
                "text": code
            }
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 60,
        "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": doc_uri },
            "position": { "line": 1, "character": 12 }
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 61,
        "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": doc_uri },
            "position": { "line": 1, "character": 12 },
            "newName": "total_count"
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 62,
        "method": "textDocument/codeAction",
        "params": {
            "textDocument": { "uri": doc_uri },
            "range": {
                "start": { "line": 1, "character": 4 },
                "end": { "line": 1, "character": 20 }
            },
            "context": {
                "diagnostics": [],
                "only": ["source.organizeImports", "refactor.extract"]
            }
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 63,
        "method": "codeAction/resolve",
        "params": {
            "title": "Organize imports",
            "kind": "source.organizeImports"
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 64,
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
    assert_eq!(responses.len(), 6);

    let prep_resp = &responses[1];
    assert_eq!(prep_resp["id"], 60);
    assert_eq!(prep_resp["result"]["placeholder"], "counter");

    let rename_resp = &responses[2];
    assert_eq!(rename_resp["id"], 61);
    let changes = rename_resp["result"]["changes"]
        .as_object()
        .expect("changes map");
    let edits = changes[doc_uri].as_array().expect("edits list");
    assert!(!edits.is_empty());
    assert_eq!(edits[0]["newText"], "total_count");

    let action_resp = &responses[3];
    assert_eq!(action_resp["id"], 62);
    let actions = action_resp["result"].as_array().expect("actions list");
    assert!(!actions.is_empty());

    let resolve_resp = &responses[4];
    assert_eq!(resolve_resp["id"], 63);
    assert_eq!(resolve_resp["result"]["title"], "Organize imports");
}

#[test]
fn test_lsp_server_stream_stage6_relationships_and_commands() {
    let mut input = Vec::new();
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {} }
    })));
    let doc_uri = "file:///tmp/stage6_test.rs";
    let code = "struct MyStruct;\nfn test_foo() {}\nfn do_work() {}\n";
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": doc_uri,
                "languageId": "rust",
                "version": 1,
                "text": code
            }
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 70,
        "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": doc_uri },
            "position": { "line": 2, "character": 4 }
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 71,
        "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": doc_uri },
            "position": { "line": 0, "character": 8 }
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 72,
        "method": "textDocument/codeLens",
        "params": {
            "textDocument": { "uri": doc_uri }
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 73,
        "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.runTest",
            "arguments": [doc_uri, "test_foo"]
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 74,
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
    assert_eq!(responses.len(), 6);

    let call_resp = &responses[1];
    assert_eq!(call_resp["id"], 70);
    let calls = call_resp["result"].as_array().expect("call array");
    assert!(!calls.is_empty());
    assert_eq!(calls[0]["name"], "do_work");

    let type_resp = &responses[2];
    assert_eq!(type_resp["id"], 71);
    let types = type_resp["result"].as_array().expect("type array");
    assert!(!types.is_empty());
    assert_eq!(types[0]["name"], "MyStruct");

    let lens_resp = &responses[3];
    assert_eq!(lens_resp["id"], 72);
    let lenses = lens_resp["result"].as_array().expect("lens array");
    assert!(!lenses.is_empty());

    let exec_resp = &responses[4];
    assert_eq!(exec_resp["id"], 73);
    assert_eq!(exec_resp["result"], Value::Null);
}

#[test]
fn test_lsp_server_stream_stage7_remaining_capabilities() {
    let mut input = Vec::new();
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {} }
    })));
    let doc_uri = "file:///tmp/stage7_test.rs";
    let code =
        "// See https://example.com/docs for details\nfn test_run() {\n    let my_var = 123;\n}\n";
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": doc_uri,
                "languageId": "rust",
                "version": 1,
                "text": code
            }
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 80,
        "method": "textDocument/documentLink",
        "params": {
            "textDocument": { "uri": doc_uri }
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 81,
        "method": "textDocument/documentColor",
        "params": {
            "textDocument": { "uri": doc_uri }
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 82,
        "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": doc_uri },
            "position": { "line": 2, "character": 10 }
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 83,
        "method": "textDocument/inlineValue",
        "params": {
            "textDocument": { "uri": doc_uri },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 3, "character": 0 }
            },
            "context": {
                "frameId": 1,
                "stoppedLocation": { "line": 2, "character": 4 }
            }
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 84,
        "method": "textDocument/inlineCompletion",
        "params": {
            "textDocument": { "uri": doc_uri },
            "position": { "line": 2, "character": 10 }
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 85,
        "method": "workspace/textDocumentContent",
        "params": {
            "uri": doc_uri
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 86,
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
    assert_eq!(responses.len(), 8);

    let link_resp = &responses[1];
    assert_eq!(link_resp["id"], 80);
    let links = link_resp["result"].as_array().expect("links array");
    assert!(!links.is_empty());
    assert_eq!(links[0]["target"], "https://example.com/docs");

    let color_resp = &responses[2];
    assert_eq!(color_resp["id"], 81);
    assert!(color_resp["result"].is_array());

    let linked_resp = &responses[3];
    assert_eq!(linked_resp["id"], 82);
    let ranges = linked_resp["result"]["ranges"]
        .as_array()
        .expect("ranges array");
    assert!(!ranges.is_empty());

    let inline_val_resp = &responses[4];
    assert_eq!(inline_val_resp["id"], 83);
    assert!(inline_val_resp["result"].is_array());

    let inline_comp_resp = &responses[5];
    assert_eq!(inline_comp_resp["id"], 84);
    assert!(inline_comp_resp["result"]["items"].is_array());

    let content_resp = &responses[6];
    assert_eq!(content_resp["id"], 85);
    assert_eq!(content_resp["result"]["text"], code);
}

#[test]
fn test_lsp_server_stream_stage8_full_workspace_dispatch() {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id_num = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp_dir = std::env::temp_dir().join(format!(
        "ridx_stream_stage8_{}_{}",
        std::process::id(),
        id_num
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    let file_path = temp_dir.join("lib.rs");
    let file_str = file_path.to_str().unwrap().to_string();
    let code = "pub struct Calculator;\n\nimpl Calculator {\n    pub fn calculate(x: i32) -> i32 {\n        x + 1\n    }\n}\n";
    std::fs::write(&file_path, code).unwrap();

    let index_file = temp_dir.join("test_crate.ridx");
    let mut writer = IndexWriter::new("test_crate", "0.1.0");
    writer.set_unit_key("unit-test-1");
    writer.set_generation("gen-test-1");
    let mut hashes = std::collections::HashMap::new();
    hashes.insert(
        file_str.clone(),
        ridx_format::normalized_source_hash(code.as_bytes()),
    );
    writer.set_file_hashes(hashes);

    writer.add_definition(RawDefinition {
        def_id: 101,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "Calculator".to_string(),
        path: "test_crate::Calculator".to_string(),
        signature: None,
        file: file_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 22,
    });
    writer.add_definition(RawDefinition {
        def_id: 100,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "calculate".to_string(),
        path: "test_crate::Calculator::calculate".to_string(),
        signature: Some("fn(i32) -> i32".to_string()),
        file: file_str.clone(),
        start_line: 4,
        start_col: 5,
        end_line: 4,
        end_col: 36,
    });
    writer.add_definition(RawDefinition {
        def_id: 102,
        kind: DefKind::Impl,
        visibility: 0,
        is_macro_expansion: false,
        name: "Calculator".to_string(),
        path: "test_crate::Calculator".to_string(),
        signature: None,
        file: file_str.clone(),
        start_line: 3,
        start_col: 1,
        end_line: 3,
        end_col: 16,
    });
    writer.add_reference(RawReference {
        file: file_str.clone(),
        start_line: 5,
        start_col: 9,
        end_line: 5,
        end_col: 10,
        target_def_id: Some(100),
        target_path: "test_crate::Calculator::calculate".to_string(),
        target_file: Some(file_str.clone()),
        target_unit: Some("unit-test-1".to_string()),
        target_line: 4,
        target_col: 12,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.add_type(RawType {
        file: file_str.clone(),
        start_line: 4,
        start_col: 22,
        end_line: 4,
        end_col: 23,
        type_name: "i32".to_string(),
    });

    let mut out_file = std::fs::File::create(&index_file).unwrap();
    writer.write_to(&mut out_file).unwrap();
    drop(out_file);

    let doc_uri = format!("file://{}", file_str);
    let mut input = Vec::new();

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "workspaceFolders": [
                { "uri": format!("file://{}", temp_dir.to_str().unwrap()), "name": "test_ws" }
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
        "method": "workspace/didChangeWorkspaceFolders",
        "params": {
            "event": {
                "added": [{ "uri": format!("file://{}", temp_dir.to_str().unwrap()), "name": "ws2" }],
                "removed": []
            }
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": doc_uri,
                "languageId": "rust",
                "version": 1,
                "text": code
            }
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": doc_uri, "version": 2 },
            "contentChanges": [{ "text": code }]
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didSave",
        "params": { "textDocument": { "uri": doc_uri } }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 10,
        "method": "textDocument/definition",
        "params": {
            "textDocument": { "uri": doc_uri },
            "position": { "line": 3, "character": 12 }
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 11,
        "method": "textDocument/declaration",
        "params": {
            "textDocument": { "uri": doc_uri },
            "position": { "line": 3, "character": 12 }
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 12,
        "method": "textDocument/typeDefinition",
        "params": {
            "textDocument": { "uri": doc_uri },
            "position": { "line": 3, "character": 12 }
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 13,
        "method": "textDocument/implementation",
        "params": {
            "textDocument": { "uri": doc_uri },
            "position": { "line": 0, "character": 12 }
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 14,
        "method": "textDocument/references",
        "params": {
            "textDocument": { "uri": doc_uri },
            "position": { "line": 3, "character": 12 },
            "context": { "includeDeclaration": true }
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 15,
        "method": "textDocument/documentHighlight",
        "params": {
            "textDocument": { "uri": doc_uri },
            "position": { "line": 3, "character": 12 }
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 16,
        "method": "workspace/symbol",
        "params": { "query": "calc" }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 17,
        "method": "workspaceSymbol/resolve",
        "params": { "name": "calculate", "kind": 12 }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 18,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": doc_uri },
            "position": { "line": 3, "character": 12 }
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 19,
        "method": "textDocument/completion",
        "params": {
            "textDocument": { "uri": doc_uri },
            "position": { "line": 3, "character": 12 }
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 20,
        "method": "completionItem/resolve",
        "params": { "label": "calculate" }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 21,
        "method": "textDocument/signatureHelp",
        "params": {
            "textDocument": { "uri": doc_uri },
            "position": { "line": 4, "character": 9 }
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 22,
        "method": "textDocument/formatting",
        "params": {
            "textDocument": { "uri": doc_uri },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 23,
        "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": doc_uri },
            "position": { "line": 3, "character": 12 }
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 24,
        "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": doc_uri },
            "position": { "line": 3, "character": 12 },
            "newName": "do_calc"
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 25,
        "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": doc_uri },
            "position": { "line": 3, "character": 12 }
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 26,
        "method": "callHierarchy/incomingCalls",
        "params": {
            "item": {
                "name": "calculate",
                "kind": 12,
                "uri": doc_uri,
                "range": {
                    "start": { "line": 3, "character": 4 },
                    "end": { "line": 3, "character": 35 }
                },
                "selectionRange": {
                    "start": { "line": 3, "character": 11 },
                    "end": { "line": 3, "character": 20 }
                }
            }
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 27,
        "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "calculate",
                "kind": 12,
                "uri": doc_uri,
                "range": {
                    "start": { "line": 3, "character": 4 },
                    "end": { "line": 3, "character": 35 }
                },
                "selectionRange": {
                    "start": { "line": 3, "character": 11 },
                    "end": { "line": 3, "character": 20 }
                }
            }
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 28,
        "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": doc_uri },
            "position": { "line": 0, "character": 12 }
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 29,
        "method": "typeHierarchy/supertypes",
        "params": {
            "item": {
                "name": "Calculator",
                "kind": 23,
                "uri": doc_uri,
                "range": {
                    "start": { "line": 0, "character": 0 },
                    "end": { "line": 0, "character": 22 }
                },
                "selectionRange": {
                    "start": { "line": 0, "character": 11 },
                    "end": { "line": 0, "character": 21 }
                }
            }
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 30,
        "method": "typeHierarchy/subtypes",
        "params": {
            "item": {
                "name": "Calculator",
                "kind": 23,
                "uri": doc_uri,
                "range": {
                    "start": { "line": 0, "character": 0 },
                    "end": { "line": 0, "character": 22 }
                },
                "selectionRange": {
                    "start": { "line": 0, "character": 11 },
                    "end": { "line": 0, "character": 21 }
                }
            }
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 31,
        "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": doc_uri },
            "positions": [{ "line": 3, "character": 12 }]
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 32,
        "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": doc_uri } }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 33,
        "method": "documentLink/resolve",
        "params": {
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 0, "character": 10 }
            },
            "target": "https://example.com"
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 34,
        "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": doc_uri } }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 35,
        "method": "textDocument/foldingRange",
        "params": { "textDocument": { "uri": doc_uri } }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 36,
        "method": "textDocument/codeAction",
        "params": {
            "textDocument": { "uri": doc_uri },
            "range": {
                "start": { "line": 3, "character": 4 },
                "end": { "line": 3, "character": 35 }
            },
            "context": { "diagnostics": [] }
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 37,
        "method": "codeAction/resolve",
        "params": { "title": "Quick Fix" }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 38,
        "method": "textDocument/codeLens",
        "params": { "textDocument": { "uri": doc_uri } }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 39,
        "method": "codeLens/resolve",
        "params": {
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 0, "character": 10 }
            }
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 40,
        "method": "textDocument/inlayHint",
        "params": {
            "textDocument": { "uri": doc_uri },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 6, "character": 0 }
            }
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 41,
        "method": "textDocument/diagnostic",
        "params": { "textDocument": { "uri": doc_uri } }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 42,
        "method": "textDocument/moniker",
        "params": {
            "textDocument": { "uri": doc_uri },
            "position": { "line": 3, "character": 12 }
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 43,
        "method": "workspace/executeCommand",
        "params": { "command": "cargo.reload" }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 44,
        "method": "textDocument/documentColor",
        "params": { "textDocument": { "uri": doc_uri } }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 45,
        "method": "textDocument/colorPresentation",
        "params": {
            "textDocument": { "uri": doc_uri },
            "color": { "red": 1.0, "green": 0.0, "blue": 0.0, "alpha": 1.0 },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 0, "character": 1 }
            }
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 46,
        "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": doc_uri },
            "position": { "line": 0, "character": 12 }
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 47,
        "method": "textDocument/inlineValue",
        "params": {
            "textDocument": { "uri": doc_uri },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 6, "character": 0 }
            },
            "context": { "frameId": 1, "stoppedLocation": { "line": 3, "character": 4 } }
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 48,
        "method": "textDocument/inlineCompletion",
        "params": {
            "textDocument": { "uri": doc_uri },
            "position": { "line": 3, "character": 12 }
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 49,
        "method": "workspace/textDocumentContent",
        "params": { "uri": doc_uri }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "workspace/didDeleteFiles",
        "params": { "files": [{ "uri": doc_uri }] }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 50,
        "method": "custom/nonExistentMethod",
        "params": {}
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didClose",
        "params": { "textDocument": { "uri": doc_uri } }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 51,
        "method": "shutdown",
        "params": null
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 52,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": doc_uri },
            "position": { "line": 0, "character": 0 }
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "exit"
    })));

    let mut output = Vec::new();
    let res = run_lsp_server_stream(Cursor::new(input), &mut output, Some(index_file));
    assert!(res.is_ok());

    let responses = parse_lsp_responses(&output);
    assert!(!responses.is_empty());

    let unknown_resp = responses.iter().find(|r| r["id"] == 50);
    assert!(unknown_resp.is_some());
    assert_eq!(unknown_resp.unwrap()["error"]["code"], -32601);

    let after_shutdown_resp = responses.iter().find(|r| r["id"] == 52);
    assert!(after_shutdown_resp.is_some());
    assert_eq!(after_shutdown_resp.unwrap()["error"]["code"], -32600);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_lsp_server_stream_sync_and_unopened_change() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_sync_test_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let src_file = temp_dir.join("src_file.rs");
    let content = "pub fn foo() {}\n";
    std::fs::write(&src_file, content).unwrap();

    let hash = ridx_format::normalized_source_hash(content.as_bytes());
    let mut writer = IndexWriter::new("sync_crate", "0.1.0");
    let mut map = std::collections::HashMap::new();
    map.insert(src_file.to_str().unwrap().to_string(), hash);
    writer.set_file_hashes(map);
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: ridx_format::DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "foo".to_string(),
        path: "sync_crate::foo".to_string(),
        signature: None,
        file: src_file.to_str().unwrap().to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 14,
    });
    let index_file = temp_dir.join("index.ridx");
    let mut f = std::fs::File::create(&index_file).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let mut ws = WorkspaceIndex::open(&index_file).unwrap();
    let mut overlay_mgr = crate::overlay::OverlayManager::new();
    let uri = format!("file://{}", src_file.to_str().unwrap());
    overlay_mgr.open_document(&uri, src_file.to_str().unwrap(), 1, content.to_string());

    // Trigger reload
    std::thread::sleep(std::time::Duration::from_millis(50));
    let mut f2 = std::fs::File::create(&index_file).unwrap();
    writer.write_to(&mut f2).unwrap();
    drop(f2);

    super::sync_reloaded_index(&mut ws, &mut overlay_mgr);

    let mut input = Vec::new();
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {}, "workspaceFolders": [{ "uri": uri, "name": "root" }] }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "initialize",
        "params": { "capabilities": {} }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeConfiguration",
        "params": { "settings": {} }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles",
        "params": { "changes": [{ "uri": uri, "type": 1 }] }
    })));
    let unopened_uri = format!("file://{}", src_file.to_str().unwrap());
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": unopened_uri, "version": 2 },
            "contentChanges": [{ "text": "pub fn foo_changed() {}\n" }]
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
    assert!(!responses.is_empty());

    let double_init = responses.iter().find(|r| r["id"] == 2);
    assert!(double_init.is_some());
    assert!(double_init.unwrap()["error"].is_object());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_lsp_server_stream_exit_before_shutdown_and_broken_pipe() {
    let mut input = Vec::new();
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "exit"
    })));
    let mut output = Vec::new();
    let res = run_lsp_server_stream(Cursor::new(input), &mut output, None);
    assert!(res.is_err());
    assert_eq!(
        res.unwrap_err().to_string(),
        "exit notification received before shutdown"
    );

    struct BrokenWriter;
    impl std::io::Write for BrokenWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "broken pipe",
            ))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "broken pipe",
            ))
        }
    }

    // 1. Uninitialized request error response fails to write
    let mut uninit_input = Vec::new();
    uninit_input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "textDocument/definition",
        "params": {}
    })));
    let res = run_lsp_server_stream(Cursor::new(uninit_input), BrokenWriter, None);
    assert!(res.is_err());

    // 2. Initialized request response fails to write
    let mut init_input = Vec::new();
    init_input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {} }
    })));
    let res2 = run_lsp_server_stream(Cursor::new(init_input), BrokenWriter, None);
    assert!(res2.is_err());

    // 3. Post-shutdown request rejection fails to write
    struct BrokenWriterAfterInit {
        writes: usize,
    }
    impl std::io::Write for BrokenWriterAfterInit {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.writes += 1;
            if self.writes > 2 {
                Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "broken pipe",
                ))
            } else {
                Ok(buf.len())
            }
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let mut shutdown_input = Vec::new();
    shutdown_input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {} }
    })));
    shutdown_input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "shutdown",
        "params": null
    })));
    shutdown_input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "textDocument/definition",
        "params": {}
    })));
    let res3 = run_lsp_server_stream(
        Cursor::new(shutdown_input),
        BrokenWriterAfterInit { writes: 0 },
        None,
    );
    assert!(res3.is_err());
}

#[test]
fn test_lsp_server_stream_folders_and_file_events() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_lsp_folders_{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let folder_uri = format!("file://{}", temp_dir.display());

    let mut input = Vec::new();
    // 1. initialize with workspaceFolders when index_path is None and multi_root is empty (line 216)
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "capabilities": {},
            "workspaceFolders": [
                { "uri": folder_uri, "name": "root_folder" }
            ]
        }
    })));

    // 2. workspace/didRenameFiles (lines 298-311)
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "workspace/didRenameFiles",
        "params": {
            "files": [
                {
                    "oldUri": format!("file://{}/src/old.rs", temp_dir.display()),
                    "newUri": format!("file://{}/src/new.rs", temp_dir.display())
                }
            ]
        }
    })));

    // 3. workspace/didDeleteFiles (lines 313-324)
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "workspace/didDeleteFiles",
        "params": {
            "files": [
                {
                    "uri": format!("file://{}/src/new.rs", temp_dir.display())
                }
            ]
        }
    })));

    // 4. shutdown and exit
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "shutdown",
        "params": null
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "exit",
        "params": null
    })));

    let mut output = Vec::new();
    let res = run_lsp_server_stream(Cursor::new(input), &mut output, None);
    assert!(res.is_ok());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_lsp_server_stream_initialize_with_empty_multi_root() {
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
        "id": 2,
        "method": "shutdown",
        "params": null
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "exit"
    })));

    let mut output = Vec::new();
    let res = run_lsp_server_stream(
        Cursor::new(input),
        &mut output,
        Some(PathBuf::from("/nonexistent/empty_mr.ridx")),
    );
    assert!(res.is_ok());
    let responses = parse_lsp_responses(&output);
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["id"], 1);
}

#[test]
fn test_sync_reloaded_index_advances_compiled_baseline() {
    let temp_dir =
        std::env::temp_dir().join(format!("ridx_sync_reload_baseline_{}", std::process::id()));
    let _ = std::fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let initial_content = "pub fn original() {}\n";
    let _ = std::fs::write(&src_file, initial_content);

    let ridx_file = temp_dir.join("index.ridx");
    let initial_hash = ridx_format::normalized_source_hash(initial_content.as_bytes());

    let mut writer = IndexWriter::new("sync_crate", "0.1.0");
    writer.get_or_insert_file(src_file.to_str().unwrap());
    writer.add_file_hash(src_file.to_str().unwrap(), initial_hash);
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "original".to_string(),
        path: "sync_crate::original".to_string(),
        signature: Some("fn()".to_string()),
        file: src_file.to_str().unwrap().to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 20,
    });
    {
        let mut f = std::fs::File::create(&ridx_file).unwrap();
        writer.write_to(&mut f).unwrap();
    }

    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let mut overlay_mgr = crate::overlay::OverlayManager::new();
    let uri = format!("file://{}", src_file.display());
    overlay_mgr.open_document(
        &uri,
        src_file.to_str().unwrap(),
        1,
        initial_content.to_string(),
    );
    assert!(!overlay_mgr
        .get(src_file.to_str().unwrap())
        .unwrap()
        .is_dirty());

    // User edits document: now dirty
    let updated_content = "pub fn original() {}\npub fn updated() {}\n";
    overlay_mgr.update(src_file.to_str().unwrap(), 2, updated_content.to_string());
    assert!(overlay_mgr
        .get(src_file.to_str().unwrap())
        .unwrap()
        .is_dirty());

    // Save edited content to disk
    let _ = std::fs::write(&src_file, updated_content);
    let updated_hash = ridx_format::normalized_source_hash(updated_content.as_bytes());

    // Rebuild and update the .ridx file with the new hash
    std::thread::sleep(std::time::Duration::from_millis(50));
    let mut writer2 = IndexWriter::new("sync_crate", "0.1.0");
    writer2.get_or_insert_file(src_file.to_str().unwrap());
    writer2.add_file_hash(src_file.to_str().unwrap(), updated_hash);
    writer2.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "updated".to_string(),
        path: "sync_crate::updated".to_string(),
        signature: Some("fn()".to_string()),
        file: src_file.to_str().unwrap().to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 2,
        end_col: 20,
    });
    {
        let mut f = std::fs::File::create(&ridx_file).unwrap();
        writer2.write_to(&mut f).unwrap();
    }

    // Now call sync_reloaded_index!
    sync_reloaded_index(&mut ws, &mut overlay_mgr);

    // Overlay compiled baseline should now be advanced, so is_dirty() is false!
    assert!(!overlay_mgr
        .get(src_file.to_str().unwrap())
        .unwrap()
        .is_dirty());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_lsp_server_stream_cancellation_and_shutdown() {
    let temp_dir =
        std::env::temp_dir().join(format!("ridx_lsp_stream_test_{}", std::process::id()));
    let _ = std::fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let _ = std::fs::write(&src_file, "pub fn stream_fn() {}\n");
    let src_uri = format!("file://{}", src_file.display());

    let mut input = Vec::new();

    // 1. Request before initialize: rejected with ServerNotInitialized (-32002)
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": src_uri },
            "position": { "line": 0, "character": 0 }
        }
    })));

    // 2. Initialize request
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "initialize",
        "params": {
            "capabilities": {},
            "rootUri": format!("file://{}", temp_dir.display())
        }
    })));

    // 3. Initialized notification
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {}
    })));

    // 4. didChange on document not opened yet (falls back to reading from disk)
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": src_uri, "version": 2 },
            "contentChanges": [{ "text": "pub fn changed_fn() {}\n" }]
        }
    })));

    // 5. Cancel request: send request with id 3, followed by $/cancelRequest with id 3
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "textDocument/definition",
        "params": {
            "textDocument": { "uri": src_uri },
            "position": { "line": 0, "character": 7 }
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "$/cancelRequest",
        "params": { "id": 3 }
    })));

    // 6. workspace/didRenameFiles
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "workspace/didRenameFiles",
        "params": {
            "files": [{
                "oldUri": format!("file://{}/old.rs", temp_dir.display()),
                "newUri": format!("file://{}/new.rs", temp_dir.display())
            }]
        }
    })));

    // 7. workspace/didDeleteFiles
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "workspace/didDeleteFiles",
        "params": {
            "files": [{
                "uri": format!("file://{}/new.rs", temp_dir.display())
            }]
        }
    })));

    // 8. shutdown request
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "shutdown",
        "params": null
    })));

    // 9. Request after shutdown: rejected with Server is shutting down (-32600)
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": src_uri },
            "position": { "line": 0, "character": 0 }
        }
    })));

    // 10. exit notification
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "exit"
    })));

    let mut output = Vec::new();
    let res = run_lsp_server_stream(Cursor::new(input), &mut output, None);
    assert!(res.is_ok());

    let responses = parse_lsp_responses(&output);
    assert_eq!(responses[0]["id"], 1);
    assert_eq!(responses[0]["error"]["code"], -32002);
    assert_eq!(responses[1]["id"], 2);
    assert!(responses[1]["result"]["capabilities"].is_object());
    assert_eq!(responses[2]["id"], 3);
    assert_eq!(responses[2]["error"]["code"], -32800);
    assert_eq!(responses[3]["id"], 4);
    assert!(responses[3]["result"].is_null());
    assert_eq!(responses[4]["id"], 5);
    assert_eq!(responses[4]["error"]["code"], -32600);

    // Also test exit notification before shutdown returns Error
    let mut exit_early_input = Vec::new();
    exit_early_input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "exit"
    })));
    let mut out_early = Vec::new();
    let res_early = run_lsp_server_stream(Cursor::new(exit_early_input), &mut out_early, None);
    assert!(res_early.is_err());
    assert!(res_early
        .unwrap_err()
        .to_string()
        .contains("exit notification received before shutdown"));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_lsp_hover_inferred_type_and_signature_help_workspace() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_lsp_type_sig_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    let file_path = temp_dir.join("lib.rs");
    let file_str = file_path.to_str().unwrap().to_string();
    let code = "pub fn caller() {\n    calculate(42);\n}\n";
    std::fs::write(&file_path, code).unwrap();

    let index_file = temp_dir.join("test_crate.ridx");
    let mut writer = IndexWriter::new("test_crate", "0.1.0");
    writer.set_unit_key("unit-sig-1");
    writer.set_generation("gen-sig-1");
    let mut hashes = std::collections::HashMap::new();
    hashes.insert(
        file_str.clone(),
        ridx_format::normalized_source_hash(code.as_bytes()),
    );
    writer.set_file_hashes(hashes);

    // Definition for calculate with signature
    writer.add_definition(RawDefinition {
        def_id: 100,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "calculate".to_string(),
        path: "test_crate::calculate".to_string(),
        signature: Some("fn(x: i32) -> i32".to_string()),
        file: file_str.clone(),
        start_line: 10,
        start_col: 1,
        end_line: 10,
        end_col: 25,
    });

    // Inferred type at line 1, col 5 (over "fn", no definition)
    writer.add_type(RawType {
        file: file_str.clone(),
        start_line: 1,
        start_col: 5,
        end_line: 1,
        end_col: 7,
        type_name: "fn() -> ()".to_string(),
    });

    let mut out_f = std::fs::File::create(&index_file).unwrap();
    writer.write_to(&mut out_f).unwrap();

    let doc_uri = format!("file://{}", file_str);
    let mut input = Vec::new();
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "rootUri": format!("file://{}", temp_dir.display()),
            "capabilities": {}
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &doc_uri,
                "languageId": "rust",
                "version": 1,
                "text": code
            }
        }
    })));

    // 1. Hover over inferred type without definition (lines 80-88 of intelligence.rs)
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 10,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": &doc_uri },
            "position": { "line": 0, "character": 5 }
        }
    })));

    // 2. Signature help inside calculate(42) finding calculate in workspace (lines 310-319 of intelligence.rs)
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 11,
        "method": "textDocument/signatureHelp",
        "params": {
            "textDocument": { "uri": &doc_uri },
            "position": { "line": 1, "character": 15 }
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 12,
        "method": "shutdown",
        "params": null
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "exit"
    })));

    let mut output = Vec::new();
    let res = run_lsp_server_stream(Cursor::new(input), &mut output, Some(index_file));
    assert!(res.is_ok());

    let responses = parse_lsp_responses(&output);
    assert_eq!(responses.len(), 4);

    let hover_resp = &responses[1];
    assert_eq!(hover_resp["id"], 10);
    assert!(hover_resp["result"]["contents"]["value"]
        .as_str()
        .unwrap()
        .contains("Type: fn() -> ()"));

    let sig_resp = &responses[2];
    assert_eq!(sig_resp["id"], 11);
    assert!(sig_resp["result"]["signatures"][0]["label"]
        .as_str()
        .unwrap()
        .contains("fn(x: i32) -> i32"));

    let _ = std::fs::remove_dir_all(&temp_dir);
}
