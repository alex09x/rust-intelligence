/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! End-to-end and protocol conformance stress tests for Domain 4:
//! Editing, Code Actions, on-type formatting, linked editing, and version conflict resolution.

use ridx::lsp::error::{CONTENT_MODIFIED, INVALID_PARAMS, REQUEST_FAILED, SERVER_NOT_INITIALIZED};
use ridx::lsp::run_lsp_server_stream;
use ridx_format::{DefKind, WorkspaceIndex};
use ridx_test_support::{make_lsp_msg, parse_lsp_responses, FixtureIndexBuilder};
use serde_json::json;
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;

#[test]
fn test_domain4_pre_initialize_requests_rejected_with_32002() {
    let mut input = Vec::new();

    // 1. Pre-init codeAction
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 101,
        "method": "textDocument/codeAction",
        "params": {
            "textDocument": { "uri": "file:///tmp/pre_init.rs" },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
            "context": { "diagnostics": [] }
        }
    })));

    // 2. Pre-init onTypeFormatting
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 102,
        "method": "textDocument/onTypeFormatting",
        "params": {
            "textDocument": { "uri": "file:///tmp/pre_init.rs" },
            "position": { "line": 0, "character": 0 },
            "ch": ";",
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    })));

    // 3. Pre-init linkedEditingRange
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 103,
        "method": "textDocument/linkedEditingRange",
        "params": {
            "textDocument": { "uri": "file:///tmp/pre_init.rs" },
            "position": { "line": 0, "character": 0 }
        }
    })));

    // 4. Initialize to cleanly terminate
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
        "method": "exit"
    })));

    let mut output = Vec::new();
    let res = run_lsp_server_stream(Cursor::new(input), &mut output, None);
    assert!(res.is_ok());

    let responses = parse_lsp_responses(&output);
    assert!(responses.len() >= 5);

    assert_eq!(responses[0]["id"], 101);
    assert_eq!(responses[0]["error"]["code"], SERVER_NOT_INITIALIZED);

    assert_eq!(responses[1]["id"], 102);
    assert_eq!(responses[1]["error"]["code"], SERVER_NOT_INITIALIZED);

    assert_eq!(responses[2]["id"], 103);
    assert_eq!(responses[2]["error"]["code"], SERVER_NOT_INITIALIZED);

    assert_eq!(responses[3]["id"], 1);
    assert!(responses[3]["result"]["capabilities"].is_object());

    assert_eq!(responses[4]["id"], 2);
}

#[test]
fn test_domain4_real_l2_external_dependency_rejection() {
    let cargo_manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ext_service_path = cargo_manifest_dir
        .join("tests/fixtures/_vendor/.cargo/registry/src/ext_service-0.5.0/src/lib.rs");

    if !ext_service_path.exists() {
        // Path safety guard
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    let local_file = temp.path().join("main.rs");
    fs::write(&local_file, "fn main() {}\n").unwrap();

    let mut builder = FixtureIndexBuilder::new("app", "0.1.0");
    let (code, _def_ids) = builder.add_defs_with_ids(
        "fn main() {}\n",
        local_file.to_str().unwrap(),
        "app",
        DefKind::Fn,
    );
    let _ = fs::write(&local_file, code);

    let ridx_file = temp.path().join("app.ridx");
    builder.write_to_file(&ridx_file);

    let ws = WorkspaceIndex::open(&ridx_file).unwrap();
    let overlay_mgr = ridx::overlay::OverlayManager::new();

    let ext_uri = format!("file://{}", ext_service_path.display());

    // 1. codeAction on external path rejected
    let req_action = json!({
        "params": {
            "textDocument": { "uri": &ext_uri },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
            "context": { "only": ["source.organizeImports"], "diagnostics": [] }
        }
    });
    let err_action =
        ridx::lsp::editing::actions::handle_code_action(&req_action, Some(&ws), &overlay_mgr)
            .unwrap_err();
    assert_eq!(err_action.code, REQUEST_FAILED);
    assert!(err_action.message.contains("external dependency"));

    // 2. codeAction/resolve targeting external path rejected
    let req_resolve = json!({
        "params": {
            "title": "Organize Imports",
            "kind": "source.organizeImports",
            "data": {
                "uri": &ext_uri,
                "version": 1,
                "kind": "source.organizeImports"
            }
        }
    });
    let err_resolve = ridx::lsp::editing::actions::handle_code_action_resolve(
        &req_resolve,
        Some(&ws),
        &overlay_mgr,
    )
    .unwrap_err();
    assert_eq!(err_resolve.code, REQUEST_FAILED);
    assert!(err_resolve.message.contains("external dependency"));
}

#[test]
fn test_domain4_stream_code_action_did_change_resolve_conflict() {
    let mut input = Vec::new();
    let uri = "file:///tmp/stream_conflict.rs";

    // 1. Initialize
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {} }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {}
    })));

    // 2. didOpen at version 1
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": uri,
                "languageId": "rust",
                "version": 1,
                "text": "use z::Z;\nuse a::A;\nfn main() {}\n"
            }
        }
    })));

    // 3. codeAction at version 1
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 10,
        "method": "textDocument/codeAction",
        "params": {
            "textDocument": { "uri": uri },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
            "context": { "only": ["source.organizeImports"], "diagnostics": [] }
        }
    })));

    // 4. didChange modifying document to version 2
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": uri, "version": 2 },
            "contentChanges": [{
                "text": "// updated\nuse z::Z;\nuse a::A;\nfn main() {}\n"
            }]
        }
    })));

    // 5. codeAction/resolve with stale action from version 1
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 20,
        "method": "codeAction/resolve",
        "params": {
            "title": "Organize Imports",
            "kind": "source.organizeImports",
            "data": {
                "uri": uri,
                "version": 1,
                "kind": "source.organizeImports"
            }
        }
    })));

    // 6. Shutdown and exit
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 99,
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

    // Response id:10 (codeAction) succeeds
    let resp_act = responses.iter().find(|r| r["id"] == 10).expect("codeAction resp");
    let actions = resp_act["result"].as_array().expect("actions array");
    assert_eq!(actions[0]["data"]["version"], 1);

    // Response id:20 (codeAction/resolve) must fail with ContentModified (-32801)
    let resp_resolve = responses.iter().find(|r| r["id"] == 20).expect("resolve resp");
    assert_eq!(
        resp_resolve["error"]["code"], CONTENT_MODIFIED,
        "Resolving stale action after didChange must emit ContentModified error (-32801)"
    );
}

#[test]
fn test_domain4_stream_execute_command() {
    let mut input = Vec::new();

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {} }
    })));

    // 1. Known command: ridx.resolveWorkspace
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 10,
        "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.resolveWorkspace",
            "rootUri": "file:///tmp/test_ws"
        }
    })));

    // 2. Unknown command: error InvalidParams (-32602)
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 11,
        "method": "workspace/executeCommand",
        "params": {
            "command": "nonexistent.command.xyz"
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

    let resp_resolve = responses.iter().find(|r| r["id"] == 10).unwrap();
    assert_eq!(resp_resolve["result"]["resolved"], true);

    let resp_unknown = responses.iter().find(|r| r["id"] == 11).unwrap();
    assert_eq!(resp_unknown["error"]["code"], INVALID_PARAMS);
}
