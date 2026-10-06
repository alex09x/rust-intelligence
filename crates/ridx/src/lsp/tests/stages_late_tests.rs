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
        "id": 855,
        "method": "initialize",
        "params": { "capabilities": {} }
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
    assert_eq!(responses.len(), 9);
    assert_eq!(responses[7]["id"], 855);
    assert_eq!(responses[7]["error"]["code"], -32600);

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
