/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::Cursor;
use std::sync::{Arc, Mutex};

fn parse_framed_json(bytes: &[u8]) -> Vec<Value> {
    let mut out = Vec::new();
    let text = String::from_utf8_lossy(bytes);
    let mut rest = &text[..];
    while let Some(pos) = rest.find("\r\n\r\n") {
        let body = &rest[pos + 4..];
        let len_header = &rest[..pos];
        let len: usize = len_header
            .lines()
            .find_map(|l| l.strip_prefix("Content-Length: "))
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        if body.len() >= len && len > 0 {
            if let Ok(val) = serde_json::from_str::<Value>(&body[..len]) {
                out.push(val);
            }
            rest = &body[len..];
        } else {
            break;
        }
    }
    out
}

#[test]
fn test_outbound_apply_edit_preserves_document_changes_version() {
    let mut overlay_mgr = OverlayManager::new();
    let file = "/tmp/test_ver.rs";
    let uri = format!("file://{file}");
    overlay_mgr.open_with_provenance(&uri, file, 1, "fn foo() {}\n".to_string(), true);

    let mut lifecycle = LifecycleManager::new();
    let mut out = Cursor::new(Vec::new());
    let mut multi_root = MultiRootWorkspace::new();
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let _ = dispatch_lsp_message(
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } }),
        &mut out,
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        None,
    );

    let cmd = json!({
        "jsonrpc": "2.0",
        "id": 100,
        "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [{
                "documentChanges": [{
                    "textDocument": { "uri": uri.clone(), "version": 1 },
                    "edits": [{
                        "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 6 } },
                        "newText": "bar"
                    }]
                }]
            }]
        }
    });

    let mut cmd_out = Cursor::new(Vec::new());
    let _ = dispatch_lsp_message(
        cmd,
        &mut cmd_out,
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        None,
    );

    let msgs = parse_framed_json(&cmd_out.into_inner());
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["method"], "workspace/applyEdit");

    let edit = &msgs[0]["params"]["edit"];
    let doc_changes = edit["documentChanges"]
        .as_array()
        .expect("Must preserve documentChanges structure");
    assert_eq!(doc_changes.len(), 1);
    assert_eq!(doc_changes[0]["textDocument"]["uri"], uri);
    assert_eq!(
        doc_changes[0]["textDocument"]["version"], 1,
        "Outbound applyEdit must preserve client document version"
    );
}

#[test]
fn test_combined_versions_retain_initial_client_snapshot() {
    let mut overlay_mgr = OverlayManager::new();
    let file = "/tmp/test_combined_ver.rs";
    let uri = format!("file://{file}");
    overlay_mgr.open_with_provenance(&uri, file, 1, "pub fn foo() {}\n".to_string(), true);

    let mut lifecycle = LifecycleManager::new();
    let mut out = Cursor::new(Vec::new());
    let mut multi_root = MultiRootWorkspace::new();
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let _ = dispatch_lsp_message(
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } }),
        &mut out,
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        None,
    );

    let cmd = json!({
        "jsonrpc": "2.0",
        "id": 300,
        "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [
                {
                    "documentChanges": [{
                        "textDocument": { "uri": uri.clone(), "version": 1 },
                        "edits": [{
                            "range": { "start": { "line": 0, "character": 7 }, "end": { "line": 0, "character": 10 } },
                            "newText": "bar"
                        }]
                    }]
                },
                {
                    "documentChanges": [{
                        "textDocument": { "uri": uri.clone(), "version": 2 },
                        "edits": [{
                            "range": { "start": { "line": 0, "character": 7 }, "end": { "line": 0, "character": 10 } },
                            "newText": "baz"
                        }]
                    }]
                }
            ]
        }
    });

    let mut cmd_out = Cursor::new(Vec::new());
    let _ = dispatch_lsp_message(
        cmd,
        &mut cmd_out,
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        None,
    );

    let msgs = parse_framed_json(&cmd_out.into_inner());
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["method"], "workspace/applyEdit");

    let edit = &msgs[0]["params"]["edit"];
    let doc_changes = edit["documentChanges"]
        .as_array()
        .expect("Must preserve documentChanges structure");
    assert_eq!(doc_changes.len(), 1);
    assert_eq!(doc_changes[0]["textDocument"]["uri"], uri);
    assert_eq!(
        doc_changes[0]["textDocument"]["version"], 1,
        "Outbound combined applyEdit must retain initial client version 1, not intermediate version 2"
    );
    let edits = doc_changes[0]["edits"].as_array().unwrap();
    assert_eq!(edits[0]["newText"], "pub fn baz() {}\n");
}

#[test]
fn test_client_advancing_to_version_2_rejects_version_1_apply_edit() {
    let mut overlay_mgr = OverlayManager::new();
    let file = "/tmp/test_client_adv.rs";
    let uri = format!("file://{file}");
    overlay_mgr.open_with_provenance(&uri, file, 1, "pub fn foo() {}\n".to_string(), true);

    let mut lifecycle = LifecycleManager::new();
    let mut out = Cursor::new(Vec::new());
    let mut multi_root = MultiRootWorkspace::new();
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let _ = dispatch_lsp_message(
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } }),
        &mut out,
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        None,
    );

    let cmd = json!({
        "jsonrpc": "2.0",
        "id": 400,
        "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [{
                "documentChanges": [{
                    "textDocument": { "uri": uri.clone(), "version": 1 },
                    "edits": [{
                        "range": { "start": { "line": 0, "character": 7 }, "end": { "line": 0, "character": 10 } },
                        "newText": "bar"
                    }]
                }]
            }]
        }
    });

    let mut cmd_out = Cursor::new(Vec::new());
    let _ = dispatch_lsp_message(
        cmd,
        &mut cmd_out,
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        None,
    );

    let msgs = parse_framed_json(&cmd_out.into_inner());
    let apply_id = msgs[0]["id"].clone();

    // Client has advanced to version 2 in editor, so client rejects the version 1 edit
    let reject_resp = json!({
        "jsonrpc": "2.0",
        "id": apply_id,
        "result": { "applied": false, "failureReason": "Document version mismatch" }
    });

    let mut resp_out = Cursor::new(Vec::new());
    let _ = dispatch_lsp_message(
        reject_resp,
        &mut resp_out,
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        None,
    );

    let resp_msgs = parse_framed_json(&resp_out.into_inner());
    assert_eq!(resp_msgs.len(), 1);
    assert_eq!(resp_msgs[0]["id"], 400);
    assert_eq!(resp_msgs[0]["result"]["applied"], false);

    // Overlay is completely untouched
    let doc = overlay_mgr.get(file).unwrap();
    assert_eq!(doc.text, "pub fn foo() {}\n");
}
