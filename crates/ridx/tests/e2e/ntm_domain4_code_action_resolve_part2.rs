/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! End-to-end tests for `codeAction/resolve` (Part 2: parameter validation, existing edit validation, data error handling).

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::error::INVALID_PARAMS;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::LspMessage;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use serde_json::{json, Value};
use std::collections::HashSet;
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

fn setup_initialized_server() -> (MultiRootWorkspace, OverlayManager, LifecycleManager, Arc<Mutex<HashSet<Value>>>) {
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

fn send_code_action_resolve_req(params: Value, mr: &mut MultiRootWorkspace, ov: &mut OverlayManager, lc: &mut LifecycleManager, can: &Arc<Mutex<HashSet<Value>>>) -> Value {
    send_code_action_resolve_req_with_id(json!(100), params, mr, ov, lc, can)
}

fn send_code_action_resolve_req_with_id(
    id: Value,
    params: Value,
    mr: &mut MultiRootWorkspace,
    ov: &mut OverlayManager,
    lc: &mut LifecycleManager,
    can: &Arc<Mutex<HashSet<Value>>>,
) -> Value {
    let mut out = Vec::new();
    let req = if params.is_null() {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "codeAction/resolve"
        })
    } else {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "codeAction/resolve",
            "params": params
        })
    };
    dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None).unwrap();
    extract_response(&out)
}

#[test]
fn test_code_action_resolve_missing_params_rejected_code_32602() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let res = send_code_action_resolve_req(Value::Null, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], INVALID_PARAMS);
    assert_eq!(res["error"]["message"], "Missing params object");
}

#[test]
fn test_code_action_resolve_non_object_params_rejected_code_32602() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let res = send_code_action_resolve_req(json!("invalid_string"), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], INVALID_PARAMS);
    assert_eq!(res["error"]["message"], "Params must be a CodeAction object");
}

#[test]
fn test_code_action_resolve_array_params_rejected_code_32602() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let res = send_code_action_resolve_req(json!([]), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], INVALID_PARAMS);
    assert_eq!(res["error"]["message"], "Params must be a CodeAction object");
}

#[test]
fn test_code_action_resolve_missing_title_rejected_code_32602() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({ "kind": "quickfix" });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], INVALID_PARAMS);
    assert_eq!(res["error"]["message"], "CodeAction must contain a title");
}

#[test]
fn test_code_action_resolve_null_title_rejected_code_32602() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({ "title": null, "kind": "quickfix" });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], INVALID_PARAMS);
    assert_eq!(res["error"]["message"], "CodeAction must contain a title");
}

#[test]
fn test_code_action_resolve_numeric_title_rejected_code_32602() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({ "title": 12345, "kind": "quickfix" });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], INVALID_PARAMS);
    assert_eq!(res["error"]["message"], "CodeAction must contain a title");
}

#[test]
fn test_code_action_resolve_no_edit_and_no_data_returns_params_unchanged() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "title": "View Document Outline",
        "kind": "quickfix"
    });
    let res = send_code_action_resolve_req(params.clone(), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"]["title"], "View Document Outline");
    assert_eq!(res["result"]["kind"], "quickfix");
    assert_eq!(res["result"]["edit"], Value::Null);
}

#[test]
fn test_code_action_resolve_with_existing_edit_returns_intact() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/valid_existing_edit.rs";
    let edit = json!({
        "changes": {
            uri: [{
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
                "newText": "// attached edit\n"
            }]
        }
    });
    let params = json!({
        "title": "Already Resolved Action",
        "kind": "quickfix",
        "edit": edit
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"]["title"], "Already Resolved Action");
    assert_eq!(res["result"]["edit"]["changes"][uri][0]["newText"], "// attached edit\n");
}

#[test]
fn test_code_action_resolve_with_existing_document_changes_edit() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/valid_doc_changes.rs";
    let edit = json!({
        "documentChanges": [{
            "textDocument": { "uri": uri, "version": 1 },
            "edits": [{
                "range": { "start": { "line": 1, "character": 0 }, "end": { "line": 1, "character": 5 } },
                "newText": "let a = 42;"
            }]
        }]
    });
    let params = json!({
        "title": "Versioned Fix",
        "kind": "quickfix",
        "edit": edit
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"]["title"], "Versioned Fix");
    assert_eq!(res["result"]["edit"]["documentChanges"][0]["textDocument"]["version"], 1);
    assert_eq!(res["result"]["edit"]["documentChanges"][0]["edits"][0]["newText"], "let a = 42;");
}

#[test]
fn test_code_action_resolve_existing_edit_read_only_dependency_rejected() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let vendor_uri = "file:///opt/vendor/.cargo/registry/src/github.com-1ecc6299db9ec823/ext-0.1.0/src/lib.rs";
    let edit = json!({
        "changes": {
            vendor_uri: [{
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
                "newText": "// illegal edit\n"
            }]
        }
    });
    let params = json!({
        "title": "Corrupt Vendor",
        "kind": "quickfix",
        "edit": edit
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], -32803);
}

#[test]
fn test_code_action_resolve_data_missing_uri_rejected_code_32602() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "title": "Defer Fix",
        "data": { "kind": "quickfix" }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], INVALID_PARAMS);
    assert_eq!(res["error"]["message"], "data missing uri");
}

#[test]
fn test_code_action_resolve_data_null_uri_rejected_code_32602() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "title": "Defer Fix",
        "data": { "uri": null, "kind": "quickfix" }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], INVALID_PARAMS);
    assert_eq!(res["error"]["message"], "data missing uri");
}

#[test]
fn test_code_action_resolve_data_unknown_kind_rejected_code_32602() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "title": "Unknown Action",
        "data": {
            "uri": "file:///tmp/test.rs",
            "kind": "unsupported.custom_refactor"
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], INVALID_PARAMS);
    assert_eq!(res["error"]["message"], "Cannot resolve CodeAction with unknown data kind: 'unsupported.custom_refactor'");
}

#[test]
fn test_code_action_resolve_quickfix_missing_range_rejected_code_32602() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "title": "Quick Fix Variable",
        "data": {
            "uri": "file:///tmp/test.rs",
            "kind": "quickfix",
            "newText": "let _ = 1;"
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], INVALID_PARAMS);
    assert_eq!(res["error"]["message"], "data missing range");
}

#[test]
fn test_code_action_resolve_refactor_extract_missing_range_rejected_code_32602() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "title": "Extract Variable",
        "data": {
            "uri": "file:///tmp/test.rs",
            "kind": "refactor.extract"
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], INVALID_PARAMS);
    assert_eq!(res["error"]["message"], "data missing range");
}

#[test]
fn test_code_action_resolve_auto_import_missing_import_path_rejected_code_32602() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "title": "Auto Import",
        "data": {
            "uri": "file:///tmp/test.rs",
            "kind": "quickfix.auto_import"
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], INVALID_PARAMS);
    assert_eq!(res["error"]["message"], "data missing import_path");
}

#[test]
fn test_code_action_resolve_response_preserves_request_id_and_jsonrpc() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "title": "Inspect Action",
        "kind": "quickfix"
    });
    let res_numeric = send_code_action_resolve_req_with_id(json!(4242), params.clone(), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res_numeric["jsonrpc"], "2.0");
    assert_eq!(res_numeric["id"], 4242);
    assert_eq!(res_numeric["result"]["title"], "Inspect Action");

    let res_str = send_code_action_resolve_req_with_id(json!("resolve-req-99"), params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res_str["jsonrpc"], "2.0");
    assert_eq!(res_str["id"], "resolve-req-99");
    assert_eq!(res_str["result"]["title"], "Inspect Action");
}
