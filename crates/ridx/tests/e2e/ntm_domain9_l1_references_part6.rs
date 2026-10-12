/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/references part 6:
//! cross-crate references, LSP dispatch, request ID preservation, and boundary edge cases.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::navigation::references::handle_references;
use ridx::lsp::protocol::{LspMessage, PositionConverter};
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use ridx_test_support::fixture::SyntheticWorkspaceFixture;
use serde_json::{json, Value};

fn make_ref_req(uri: &str, line: u32, col: u32, include_decl: bool) -> Value {
    json!({
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": col },
            "context": { "includeDeclaration": include_decl }
        }
    })
}

fn extract_response(out: &[u8]) -> serde_json::Value {
    let s = String::from_utf8_lossy(out);
    for part in s.split("Content-Length: ") {
        if let Some(idx) = part.find("\r\n\r\n") {
            let body = &part[idx + 4..];
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(body.trim()) {
                return v;
            }
        }
    }
    serde_json::Value::Null
}

fn setup_initialized_server() -> (
    MultiRootWorkspace,
    OverlayManager,
    LifecycleManager,
    Arc<Mutex<HashSet<serde_json::Value>>>,
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

#[test]
fn test_references_cross_crate_downstream_dependency() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("core_lib", "0.1.0").unwrap();
    ws.add_crate("app_cli", "0.1.0").unwrap();

    let (_, ids) = ws
        .add_source_file_with_defs("core_lib", "src/lib.rs", "pub struct «CoreToken»;\n", DefKind::Struct)
        .unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_refs_to_foreign_def("app_cli", "src/main.rs", "use core_lib::CoreToken;\npub fn run() { let _ = «CoreToken»; }\n", "core_lib", ids[0]).unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let core_uri = ws.file_uri("core_lib", "src/lib.rs").unwrap();
    let app_uri = ws.file_uri("app_cli", "src/main.rs").unwrap();

    let req = make_ref_req(&core_uri, 0, 11, false);
    let res = handle_references(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["uri"], app_uri);
}

#[test]
fn test_references_cross_crate_multiple_downstream_crates() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("shared_model", "0.1.0").unwrap();
    ws.add_crate("consumer_one", "0.1.0").unwrap();
    ws.add_crate("consumer_two", "0.1.0").unwrap();

    let (_, ids) = ws
        .add_source_file_with_defs("shared_model", "src/lib.rs", "pub struct «GlobalItem»;\n", DefKind::Struct)
        .unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_refs_to_foreign_def("consumer_one", "src/lib.rs", "«GlobalItem»;\n", "shared_model", ids[0]).unwrap();
    ws.add_refs_to_foreign_def("consumer_two", "src/lib.rs", "«GlobalItem»;\n", "shared_model", ids[0]).unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let shared_uri = ws.file_uri("shared_model", "src/lib.rs").unwrap();
    let one_uri = ws.file_uri("consumer_one", "src/lib.rs").unwrap();
    let two_uri = ws.file_uri("consumer_two", "src/lib.rs").unwrap();

    let req = make_ref_req(&shared_uri, 0, 11, false);
    let res = handle_references(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    let mut uris: Vec<_> = arr.iter().map(|l| l["uri"].as_str().unwrap()).collect();
    uris.sort();
    let mut expected = vec![one_uri.as_str(), two_uri.as_str()];
    expected.sort();
    assert_eq!(uris, expected);
}

#[test]
fn test_references_preserves_numeric_request_id() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 10001,
        "method": "textDocument/references",
        "params": {
            "textDocument": { "uri": "file:///tmp/unregistered.rs" },
            "position": { "line": 0, "character": 0 },
            "context": { "includeDeclaration": true }
        }
    });
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, &mut lc, &mut mr, &mut ov, &can, None);
    let resp = extract_response(&out);
    assert_eq!(resp["id"], 10001);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_references_preserves_string_request_id() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": "req-refs-str-1",
        "method": "textDocument/references",
        "params": {
            "textDocument": { "uri": "file:///tmp/unregistered.rs" },
            "position": { "line": 0, "character": 0 },
            "context": { "includeDeclaration": false }
        }
    });
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, &mut lc, &mut mr, &mut ov, &can, None);
    let resp = extract_response(&out);
    assert_eq!(resp["id"], "req-refs-str-1");
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_references_preserves_large_numeric_id() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 77777777,
        "method": "textDocument/references",
        "params": {
            "textDocument": { "uri": "file:///tmp/unregistered.rs" },
            "position": { "line": 0, "character": 0 },
            "context": { "includeDeclaration": true }
        }
    });
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, &mut lc, &mut mr, &mut ov, &can, None);
    let resp = extract_response(&out);
    assert_eq!(resp["id"], 77777777);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_references_whitespace_cursor_returns_empty_array() {
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let req = make_ref_req("file:///tmp/empty.rs", 0, 0, true);
    let res = handle_references(&req, None, &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]));
}

#[test]
fn test_references_unregistered_uri_returns_empty_array() {
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let req = make_ref_req("file:///not/indexed.rs", 5, 5, true);
    let res = handle_references(&req, None, &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]));
}

#[test]
fn test_references_out_of_bounds_line_returns_empty_array() {
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let req = make_ref_req("file:///tmp/file.rs", 99999, 0, true);
    let res = handle_references(&req, None, &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]));
}

#[test]
fn test_references_out_of_bounds_col_returns_empty_array() {
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let req = make_ref_req("file:///tmp/file.rs", 0, 99999, true);
    let res = handle_references(&req, None, &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]));
}

#[test]
fn test_references_no_workspace_returns_empty_array() {
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let req = make_ref_req("file:///tmp/file.rs", 1, 1, false);
    let res = handle_references(&req, None, &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]));
}

#[test]
fn test_references_empty_params_safe_handling() {
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let req = json!({
        "params": {}
    });
    let res = handle_references(&req, None, &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]));
}
