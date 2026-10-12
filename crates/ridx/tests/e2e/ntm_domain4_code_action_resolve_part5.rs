/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! End-to-end tests for `codeAction/resolve` (Part 5: extract variable, auto-import, multiroot routing, and command preservation).

use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::protocol::LspMessage;
use ridx::overlay::OverlayManager;
use ridx::lsp::workspace::MultiRootWorkspace;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::sync::{Arc, Mutex};

fn extract_response(out: &[u8]) -> Value {
    let s = String::from_utf8_lossy(out);
    for part in s.split("Content-Length: ") {
        if let Some(idx) = part.find("\r\n\r\n") {
            let body = &part[idx + 4..];
            if let Ok(v) = serde_json::from_str::<Value>(body.trim()) {
                return v;
            }
        }
    }
    Value::Null
}

fn setup_initialized_server() -> (
    MultiRootWorkspace,
    OverlayManager,
    LifecycleManager,
    Arc<Mutex<HashSet<Value>>>,
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

fn send_code_action_resolve_req(
    params: Value,
    mr: &mut MultiRootWorkspace,
    ov: &mut OverlayManager,
    lc: &mut LifecycleManager,
    can: &Arc<Mutex<HashSet<Value>>>,
) -> Value {
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 100,
        "method": "codeAction/resolve",
        "params": params
    });
    dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None).unwrap();
    extract_response(&out)
}

#[test]
fn test_code_action_resolve_extract_variable_valid_expression() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/extract_var.rs";
    let file = "/tmp/extract_var.rs";
    let content = "fn foo() {\n    let a = 1 + 2;\n}\n";
    ov.open_with_provenance(uri, file, 1, content.to_string(), true);

    let params = json!({
        "title": "Extract variable `var_name`",
        "kind": "refactor.extract",
        "data": {
            "uri": uri,
            "version": 1,
            "kind": "refactor.extract",
            "range": { "start": { "line": 1, "character": 12 }, "end": { "line": 1, "character": 17 } }
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"]["title"], "Extract variable `var_name`");
    let edits = &res["result"]["edit"]["documentChanges"][0]["edits"];
    assert_eq!(edits[0]["newText"], "    let extracted = 1 + 2;\n");
    assert_eq!(edits[1]["newText"], "extracted");
}

#[test]
fn test_code_action_resolve_extract_variable_invalid_range_request_failed() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/extract_invalid.rs";
    let file = "/tmp/extract_invalid.rs";
    ov.open_with_provenance(uri, file, 1, "fn foo() {}\n".to_string(), true);

    let params = json!({
        "title": "Extract variable",
        "kind": "refactor.extract",
        "data": {
            "uri": uri,
            "version": 1,
            "kind": "refactor.extract",
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 2 } }
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], -32803);
    assert_eq!(res["error"]["message"], "Cannot extract variable at range");
}

#[test]
fn test_code_action_resolve_extract_variable_preserves_document_version() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/extract_ver.rs";
    let file = "/tmp/extract_ver.rs";
    let content = "fn foo() {\n    let a = 10 * 20;\n}\n";
    ov.open_with_provenance(uri, file, 4, content.to_string(), true);

    let params = json!({
        "title": "Extract variable `var_name`",
        "kind": "refactor.extract",
        "data": {
            "uri": uri,
            "version": 4,
            "kind": "refactor.extract",
            "range": { "start": { "line": 1, "character": 12 }, "end": { "line": 1, "character": 19 } }
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(
        res["result"]["edit"]["documentChanges"][0]["textDocument"]["version"],
        4
    );
}

#[test]
fn test_code_action_resolve_extract_variable_disk_fallback() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("extract_disk.rs");
    fs::write(&file, "fn bar() {\n    let x = 40 + 2;\n}\n").unwrap();

    let uri = format!("file://{}", file.display());
    let params = json!({
        "title": "Extract variable `var_name`",
        "kind": "refactor.extract",
        "data": {
            "uri": uri,
            "kind": "refactor.extract",
            "range": { "start": { "line": 1, "character": 12 }, "end": { "line": 1, "character": 18 } }
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"]["title"], "Extract variable `var_name`");
    let edits = &res["result"]["edit"]["changes"][&uri];
    assert_eq!(edits[0]["newText"], "    let extracted = 40 + 2;\n");
    assert_eq!(edits[1]["newText"], "extracted");
}

#[test]
fn test_code_action_resolve_auto_import_resolves_text_edit() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/auto_import.rs";
    let file = "/tmp/auto_import.rs";
    ov.open_with_provenance(uri, file, 2, "fn main() {}\n".to_string(), true);

    let params = json!({
        "title": "Import `std::collections::HashMap`",
        "kind": "quickfix",
        "data": {
            "uri": uri,
            "version": 2,
            "kind": "quickfix.auto_import",
            "import_path": "std::collections::HashMap"
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"]["title"], "Import `std::collections::HashMap`");
    let doc_changes = &res["result"]["edit"]["documentChanges"][0];
    assert_eq!(doc_changes["textDocument"]["version"], 2);
    assert_eq!(
        doc_changes["edits"][0]["newText"],
        "use std::collections::HashMap;\n"
    );
    assert_eq!(doc_changes["edits"][0]["range"]["start"]["line"], 0);
}

#[test]
fn test_code_action_resolve_auto_import_preserves_document_version() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/auto_import_ver.rs";
    let file = "/tmp/auto_import_ver.rs";
    ov.open_with_provenance(uri, file, 9, "fn main() {}\n".to_string(), true);

    let params = json!({
        "title": "Import `std::sync::Arc`",
        "kind": "quickfix",
        "data": {
            "uri": uri,
            "version": 9,
            "kind": "quickfix.auto_import",
            "import_path": "std::sync::Arc"
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(
        res["result"]["edit"]["documentChanges"][0]["textDocument"]["version"],
        9
    );
}

#[test]
fn test_code_action_resolve_auto_import_nested_path() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/auto_import_nested.rs";
    let params = json!({
        "title": "Import `crate::utils::helpers::Helper`",
        "kind": "quickfix",
        "data": {
            "uri": uri,
            "kind": "quickfix.auto_import",
            "import_path": "crate::utils::helpers::Helper"
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    let edit = &res["result"]["edit"]["changes"][uri][0];
    assert_eq!(edit["newText"], "use crate::utils::helpers::Helper;\n");
}

#[test]
fn test_code_action_resolve_multi_root_workspace_routing() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri1 = "file:///workspace_a/src/lib.rs";
    let params = json!({
        "title": "Fix Workspace A",
        "kind": "quickfix",
        "data": {
            "uri": uri1,
            "kind": "quickfix",
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
            "newText": "// fix a\n"
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"]["title"], "Fix Workspace A");
    assert_eq!(res["result"]["edit"]["changes"][uri1][0]["newText"], "// fix a\n");
}

#[test]
fn test_code_action_resolve_idempotent_multiple_resolves() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/idempotent_resolve.rs";
    let params = json!({
        "title": "Quick Fix Ident",
        "kind": "quickfix",
        "data": {
            "uri": uri,
            "kind": "quickfix",
            "range": { "start": { "line": 0, "character": 4 }, "end": { "line": 0, "character": 10 } },
            "newText": "_ident"
        }
    });
    let res1 = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res1["result"]["title"], "Quick Fix Ident");
    let resolved_action = res1["result"].clone();

    // Resolving already-resolved action returns identical result
    let res2 = send_code_action_resolve_req(resolved_action, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res2["result"]["title"], "Quick Fix Ident");
    assert_eq!(
        res2["result"]["edit"]["changes"][uri][0]["newText"],
        res1["result"]["edit"]["changes"][uri][0]["newText"]
    );
}

#[test]
fn test_code_action_resolve_preserves_client_command_field() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/command_preserved.rs";
    let command = json!({
        "title": "Trigger Flycheck",
        "command": "rust.triggerFlycheck",
        "arguments": ["build"]
    });
    let params = json!({
        "title": "Fix With Command",
        "kind": "quickfix",
        "command": command.clone(),
        "data": {
            "uri": uri,
            "kind": "quickfix",
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
            "newText": "// command fix\n"
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"]["title"], "Fix With Command");
    assert_eq!(res["result"]["command"]["title"], "Trigger Flycheck");
    assert_eq!(res["result"]["command"]["command"], "rust.triggerFlycheck");
    assert_eq!(res["result"]["command"]["arguments"][0], "build");
    assert_eq!(res["result"]["edit"]["changes"][uri][0]["newText"], "// command fix\n");
}
