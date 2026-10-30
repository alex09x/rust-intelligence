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
use serde_json::json;
use std::io::Cursor;

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
    let items = comp_resp["result"]["items"].as_array().expect("items array");
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
fn test_lsp_server_stream_file_rename_delete_didchange_unopened() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_rename_del_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let f1 = temp_dir.join("f1.rs");
    let f2 = temp_dir.join("f2.rs");
    std::fs::write(&f1, "pub fn f1() {}\n").unwrap();

    let uri1 = format!("file://{}", f1.to_str().unwrap());
    let uri2 = format!("file://{}", f2.to_str().unwrap());

    let mut input = Vec::new();
    // Request before initialize (line 363 in dispatch.rs, error -32002)
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 10,
        "method": "textDocument/hover",
        "params": { "textDocument": { "uri": &uri1 }, "position": { "line": 0, "character": 0 } }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {} }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "initialized"
    })));
    // Cancelled request (lines 300-305, 308 in dispatch.rs, error -32800)
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "$/cancelRequest",
        "params": { "id": 20 }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 20,
        "method": "textDocument/hover",
        "params": { "textDocument": { "uri": &uri1 }, "position": { "line": 0, "character": 0 } }
    })));
    // didChange on unopened document f1 (lines 121-127 in dispatch.rs)
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": &uri1, "version": 2 },
            "contentChanges": [{ "text": "pub fn f1_updated() {}\n" }]
        }
    })));
    // workspace/didRenameFiles (lines 155-165 in dispatch.rs)
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "workspace/didRenameFiles",
        "params": {
            "files": [{ "oldUri": &uri1, "newUri": &uri2 }]
        }
    })));
    // workspace/didDeleteFiles (lines 167-176 in dispatch.rs)
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "workspace/didDeleteFiles",
        "params": {
            "files": [{ "uri": &uri2 }]
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "shutdown",
        "params": null
    })));
    // Request after shutdown (line 356 in dispatch.rs, error -32600)
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 30,
        "method": "textDocument/hover",
        "params": { "textDocument": { "uri": &uri1 }, "position": { "line": 0, "character": 0 } }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "exit"
    })));

    let mut output = Vec::new();
    let res = run_lsp_server_stream(Cursor::new(input), &mut output, None);
    assert!(res.is_ok());

    let _ = std::fs::remove_dir_all(&temp_dir);
}
