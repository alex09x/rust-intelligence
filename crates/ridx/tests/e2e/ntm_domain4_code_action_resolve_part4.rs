/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! End-to-end tests for `codeAction/resolve` (Part 4: quickfix text edit construction and organize imports resolution).

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
fn test_code_action_resolve_quickfix_simple_variable_prefix() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/quickfix_var.rs";
    let params = json!({
        "title": "Prefix `unused` with underscore",
        "kind": "quickfix",
        "data": {
            "uri": uri,
            "kind": "quickfix",
            "range": { "start": { "line": 1, "character": 8 }, "end": { "line": 1, "character": 14 } },
            "newText": "_unused"
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    let edit = &res["result"]["edit"]["changes"][uri][0];
    assert_eq!(edit["range"]["start"]["line"], 1);
    assert_eq!(edit["range"]["start"]["character"], 8);
    assert_eq!(edit["range"]["end"]["character"], 14);
    assert_eq!(edit["newText"], "_unused");
}

#[test]
fn test_code_action_resolve_quickfix_remove_mut() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/quickfix_mut.rs";
    let params = json!({
        "title": "Remove unused `mut`",
        "kind": "quickfix",
        "data": {
            "uri": uri,
            "kind": "quickfix",
            "range": { "start": { "line": 2, "character": 4 }, "end": { "line": 2, "character": 8 } },
            "newText": ""
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    let edit = &res["result"]["edit"]["changes"][uri][0];
    assert_eq!(edit["range"]["start"]["line"], 2);
    assert_eq!(edit["range"]["start"]["character"], 4);
    assert_eq!(edit["range"]["end"]["character"], 8);
    assert_eq!(edit["newText"], "");
}

#[test]
fn test_code_action_resolve_quickfix_empty_new_text_valid() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/quickfix_empty.rs";
    let params = json!({
        "title": "Delete statement",
        "kind": "quickfix",
        "data": {
            "uri": uri,
            "kind": "quickfix",
            "range": { "start": { "line": 5, "character": 0 }, "end": { "line": 6, "character": 0 } },
            "newText": ""
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"]["edit"]["changes"][uri][0]["newText"], "");
}

#[test]
fn test_code_action_resolve_quickfix_multiline_replacement() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/quickfix_multi.rs";
    let new_code = "    let x = 10;\n    let y = 20;\n";
    let params = json!({
        "title": "Split declaration",
        "kind": "quickfix",
        "data": {
            "uri": uri,
            "kind": "quickfix",
            "range": { "start": { "line": 3, "character": 0 }, "end": { "line": 4, "character": 0 } },
            "newText": new_code
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"]["edit"]["changes"][uri][0]["newText"], new_code);
}

#[test]
fn test_code_action_resolve_quickfix_preserves_action_diagnostics_field() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/quickfix_diag.rs";
    let diag = json!({
        "range": { "start": { "line": 0, "character": 4 }, "end": { "line": 0, "character": 10 } },
        "message": "unused variable: `unused`",
        "code": "unused_variables"
    });
    let params = json!({
        "title": "Prefix with underscore",
        "kind": "quickfix",
        "diagnostics": [diag.clone()],
        "data": {
            "uri": uri,
            "kind": "quickfix",
            "range": { "start": { "line": 0, "character": 4 }, "end": { "line": 0, "character": 10 } },
            "newText": "_unused"
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"]["diagnostics"][0]["code"], "unused_variables");
    assert_eq!(res["result"]["diagnostics"][0]["message"], "unused variable: `unused`");
    assert_eq!(res["result"]["edit"]["changes"][uri][0]["newText"], "_unused");
}

#[test]
fn test_code_action_resolve_organize_imports_resolves_text_edit() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/organize_imports.rs";
    let file = "/tmp/organize_imports.rs";
    ov.open_with_provenance(uri, file, 1, "use b::B;\nuse a::A;\n".to_string(), true);

    let params = json!({
        "title": "Organize Imports",
        "kind": "source.organizeImports",
        "data": {
            "uri": uri,
            "version": 1,
            "kind": "source.organizeImports"
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"]["title"], "Organize Imports");
    let doc_changes = &res["result"]["edit"]["documentChanges"][0];
    assert_eq!(doc_changes["textDocument"]["version"], 1);
    assert_eq!(doc_changes["edits"][0]["newText"], "use a::A;\nuse b::B;");
}

#[test]
fn test_code_action_resolve_organize_imports_with_no_imports_request_failed() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/no_imports.rs";
    let file = "/tmp/no_imports.rs";
    ov.open_with_provenance(uri, file, 1, "fn main() {}\n".to_string(), true);

    let params = json!({
        "title": "Organize Imports",
        "kind": "source.organizeImports",
        "data": {
            "uri": uri,
            "version": 1,
            "kind": "source.organizeImports"
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], -32803);
    assert_eq!(res["error"]["message"], "No imports to organize");
}

#[test]
fn test_code_action_resolve_organize_imports_preserves_document_version() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/organize_ver.rs";
    let file = "/tmp/organize_ver.rs";
    ov.open_with_provenance(uri, file, 7, "use z::Z;\nuse y::Y;\n".to_string(), true);

    let params = json!({
        "title": "Organize Imports",
        "kind": "source.organizeImports",
        "data": {
            "uri": uri,
            "version": 7,
            "kind": "source.organizeImports"
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(
        res["result"]["edit"]["documentChanges"][0]["textDocument"]["version"],
        7
    );
}

#[test]
fn test_code_action_resolve_organize_imports_disk_fallback_has_no_version() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("organize_disk.rs");
    fs::write(&file, "use d::D;\nuse c::C;\n").unwrap();

    let uri = format!("file://{}", file.display());
    let params = json!({
        "title": "Organize Imports",
        "kind": "source.organizeImports",
        "data": {
            "uri": uri,
            "kind": "source.organizeImports"
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"]["edit"]["changes"][&uri][0]["newText"], "use c::C;\nuse d::D;");
}

#[test]
fn test_code_action_resolve_organize_imports_deduplicates_duplicate_use() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/dedup_use.rs";
    let file = "/tmp/dedup_use.rs";
    ov.open_with_provenance(uri, file, 1, "use a::A;\nuse a::A;\n".to_string(), true);

    let params = json!({
        "title": "Organize Imports",
        "kind": "source.organizeImports",
        "data": {
            "uri": uri,
            "version": 1,
            "kind": "source.organizeImports"
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    let new_text = res["result"]["edit"]["documentChanges"][0]["edits"][0]["newText"].as_str().unwrap();
    assert_eq!(new_text, "use a::A;");
}

#[test]
fn test_code_action_resolve_organize_imports_with_leading_comments() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/commented_use.rs";
    let file = "/tmp/commented_use.rs";
    let content = "// Important comment\nuse b::B;\nuse a::A;\n";
    ov.open_with_provenance(uri, file, 1, content.to_string(), true);

    let params = json!({
        "title": "Organize Imports",
        "kind": "source.organizeImports",
        "data": {
            "uri": uri,
            "version": 1,
            "kind": "source.organizeImports"
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    let new_text = res["result"]["edit"]["documentChanges"][0]["edits"][0]["newText"].as_str().unwrap();
    assert_eq!(new_text, "// Important comment\nuse a::A;\nuse b::B;");
}
