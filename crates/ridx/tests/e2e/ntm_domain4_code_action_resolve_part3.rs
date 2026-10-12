/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! End-to-end tests for `codeAction/resolve` (Part 3: staleness/version conflict safety -32801, cancellation, and disk fallback).

use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::error::CONTENT_MODIFIED;
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
    let req = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "codeAction/resolve",
        "params": params
    });
    dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None).unwrap();
    extract_response(&out)
}

#[test]
fn test_code_action_resolve_overlay_version_match_succeeds() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/version_match.rs";
    let file = "/tmp/version_match.rs";
    ov.open_with_provenance(uri, file, 2, "let mut x = 1;\n".to_string(), true);

    let params = json!({
        "title": "Remove unused mut",
        "data": {
            "uri": uri,
            "version": 2,
            "kind": "quickfix",
            "range": { "start": { "line": 0, "character": 4 }, "end": { "line": 0, "character": 8 } },
            "newText": ""
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"]["title"], "Remove unused mut");
    assert_eq!(res["result"]["edit"]["changes"][uri][0]["newText"], "");
}

#[test]
fn test_code_action_resolve_overlay_version_mismatch_rejected_content_modified_32801() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/version_mismatch.rs";
    let file = "/tmp/version_mismatch.rs";
    ov.open_with_provenance(uri, file, 3, "let mut x = 1;\n".to_string(), true);

    let params = json!({
        "title": "Stale Action",
        "data": {
            "uri": uri,
            "version": 2,
            "kind": "quickfix",
            "range": { "start": { "line": 0, "character": 4 }, "end": { "line": 0, "character": 8 } },
            "newText": ""
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], CONTENT_MODIFIED);
    assert_eq!(
        res["error"]["message"],
        "Document 'file:///tmp/version_mismatch.rs' was modified since code action was computed (version 3 != expected 2)"
    );
}

#[test]
fn test_code_action_resolve_overlay_advanced_twice_rejected_32801() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/version_advanced.rs";
    let file = "/tmp/version_advanced.rs";
    ov.open_with_provenance(uri, file, 5, "let mut x = 1;\n".to_string(), true);

    let params = json!({
        "title": "Stale Action",
        "data": {
            "uri": uri,
            "version": 2,
            "kind": "quickfix",
            "range": { "start": { "line": 0, "character": 4 }, "end": { "line": 0, "character": 8 } },
            "newText": ""
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], CONTENT_MODIFIED);
}

#[test]
fn test_code_action_resolve_overlay_regression_rejected_32801() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/version_regression.rs";
    let file = "/tmp/version_regression.rs";
    ov.open_with_provenance(uri, file, 1, "let mut x = 1;\n".to_string(), true);

    let params = json!({
        "title": "Future Action",
        "data": {
            "uri": uri,
            "version": 2,
            "kind": "quickfix",
            "range": { "start": { "line": 0, "character": 4 }, "end": { "line": 0, "character": 8 } },
            "newText": ""
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], CONTENT_MODIFIED);
}

#[test]
fn test_code_action_resolve_disk_content_hash_match_succeeds() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("hash_match.rs");
    let content = "let unused_var = 10;\n";
    fs::write(&file, content).unwrap();

    let disk_bytes = fs::read(&file).unwrap();
    let hash = ridx_format::normalized_source_hash(&disk_bytes);
    let uri = format!("file://{}", file.display());

    let params = json!({
        "title": "Prefix with underscore",
        "data": {
            "uri": uri,
            "content_hash": hash,
            "kind": "quickfix",
            "range": { "start": { "line": 0, "character": 4 }, "end": { "line": 0, "character": 14 } },
            "newText": "_unused_var"
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"]["title"], "Prefix with underscore");
    assert_eq!(res["result"]["edit"]["changes"][&uri][0]["newText"], "_unused_var");
}

#[test]
fn test_code_action_resolve_disk_content_hash_mismatch_rejected_32801() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("hash_mismatch.rs");
    fs::write(&file, "let x = 1;\n").unwrap();

    let uri = format!("file://{}", file.display());
    let params = json!({
        "title": "Stale Disk Action",
        "data": {
            "uri": uri,
            "content_hash": 9999999999u64,
            "kind": "quickfix",
            "range": { "start": { "line": 0, "character": 4 }, "end": { "line": 0, "character": 5 } },
            "newText": "_x"
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], CONTENT_MODIFIED);
    assert_eq!(
        res["error"]["message"],
        format!("Document '{uri}' on disk was modified since code action was computed")
    );
}

#[test]
fn test_code_action_resolve_non_existent_file_disk_hash_mismatch_rejected_32801() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/definitely_absent_123456.rs";
    let params = json!({
        "title": "Missing File Action",
        "data": {
            "uri": uri,
            "content_hash": 12345678u64,
            "kind": "quickfix",
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } },
            "newText": "_x"
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], CONTENT_MODIFIED);
}

#[test]
fn test_code_action_resolve_cancellation_removes_request_cleanly() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    can.lock().unwrap().insert(json!(777));

    let params = json!({
        "title": "Cancelled Action",
        "kind": "quickfix"
    });
    let res = send_code_action_resolve_req_with_id(json!(777), params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], -32800);
}

#[test]
fn test_code_action_resolve_read_only_dependency_in_data_rejected() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let vendor_uri = "file:///opt/vendor/.cargo/registry/src/github.com-1ecc6299db9ec823/tokio-1.0.0/src/lib.rs";
    let params = json!({
        "title": "Modify Vendor Lib",
        "data": {
            "uri": vendor_uri,
            "kind": "quickfix",
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
            "newText": "// illegal\n"
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], -32803);
}

#[test]
fn test_code_action_resolve_closed_document_uses_disk_content() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("closed_file.rs");
    let content = "let unused = 5;\n";
    fs::write(&file, content).unwrap();

    let uri = format!("file://{}", file.display());
    let file_str = file.to_str().unwrap();
    ov.open_with_provenance(&uri, file_str, 1, content.to_string(), true);
    ov.close(file_str);
    assert!(ov.get(file_str).is_none());

    let params = json!({
        "title": "Prefix with underscore",
        "data": {
            "uri": uri,
            "kind": "quickfix",
            "range": { "start": { "line": 0, "character": 4 }, "end": { "line": 0, "character": 10 } },
            "newText": "_unused"
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["result"]["title"], "Prefix with underscore");
    assert_eq!(res["result"]["edit"]["changes"][&uri][0]["newText"], "_unused");
}

#[test]
fn test_code_action_resolve_reopened_document_new_version_rejected_32801() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/reopen_test.rs";
    let file = "/tmp/reopen_test.rs";

    ov.open_with_provenance(uri, file, 1, "let x = 1;\n".to_string(), true);
    ov.close(file);
    ov.open_with_provenance(uri, file, 2, "let x = 2;\n".to_string(), true);

    let params = json!({
        "title": "Old Version Fix",
        "data": {
            "uri": uri,
            "version": 1,
            "kind": "quickfix",
            "range": { "start": { "line": 0, "character": 4 }, "end": { "line": 0, "character": 5 } },
            "newText": "_x"
        }
    });
    let res = send_code_action_resolve_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], CONTENT_MODIFIED);
}
