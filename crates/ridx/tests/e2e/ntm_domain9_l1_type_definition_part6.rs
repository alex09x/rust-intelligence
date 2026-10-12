/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/typeDefinition part 6:
//! cross-crate resolution, LSP dispatch, ID preservation, and boundary edge cases.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::navigation::handle_type_definition;
use ridx::lsp::protocol::{LspMessage, PositionConverter};
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use ridx_test_support::fixture::SyntheticWorkspaceFixture;
use serde_json::{json, Value};

fn make_pos_req(uri: &str, line: u32, col: u32) -> Value {
    json!({
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": col }
        }
    })
}

fn assert_range(range: &Value, start_line: u32, start_col: u32, end_line: u32, end_col: u32) {
    assert_eq!(range["start"]["line"], start_line);
    assert_eq!(range["start"]["character"], start_col);
    assert_eq!(range["end"]["line"], end_line);
    assert_eq!(range["end"]["character"], end_col);
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
fn test_type_definition_cross_crate_dependency() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("dep_crate", "0.1.0").unwrap();
    ws.add_crate("app_crate", "0.1.0").unwrap();

    let (_, ids) = ws
        .add_source_file_with_defs("dep_crate", "src/lib.rs", "pub struct «ServiceConfig»;\n", DefKind::Struct)
        .unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file("app_crate", "src/lib.rs", "pub fn start() { let cfg = 0; }\n").unwrap();
    ws.add_type_at("app_crate", "src/lib.rs", 1, 21, 1, 24, "ServiceConfig").unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let app_uri = ws.file_uri("app_crate", "src/lib.rs").unwrap();
    let dep_uri = ws.file_uri("dep_crate", "src/lib.rs").unwrap();

    let req = make_pos_req(&app_uri, 0, 22);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res["uri"], dep_uri);
    assert_range(&res["range"], 0, 11, 0, 24);
}

#[test]
fn test_type_definition_cross_crate_reexport() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("core_backend", "0.1.0").unwrap();
    ws.add_crate("client_fe", "0.1.0").unwrap();

    let (_, ids) = ws
        .add_source_file_with_defs("core_backend", "src/lib.rs", "pub struct «DatabasePool»;\n", DefKind::Struct)
        .unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file("client_fe", "src/lib.rs", "pub fn connect() { let pool = 0; }\n").unwrap();
    ws.add_type_at("client_fe", "src/lib.rs", 1, 23, 1, 27, "DatabasePool").unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let fe_uri = ws.file_uri("client_fe", "src/lib.rs").unwrap();
    let be_uri = ws.file_uri("core_backend", "src/lib.rs").unwrap();

    let req = make_pos_req(&fe_uri, 0, 24);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res["uri"], be_uri);
    assert_range(&res["range"], 0, 11, 0, 23);
}

#[test]
fn test_type_definition_preserves_numeric_request_id() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 8001,
        "method": "textDocument/typeDefinition",
        "params": {
            "textDocument": { "uri": "file:///tmp/unregistered.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, &mut lc, &mut mr, &mut ov, &can, None);
    let resp = extract_response(&out);
    assert_eq!(resp["id"], 8001);
    assert_eq!(resp["result"], json!(null));
}

#[test]
fn test_type_definition_preserves_string_request_id() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": "req-typedef-str-1",
        "method": "textDocument/typeDefinition",
        "params": {
            "textDocument": { "uri": "file:///tmp/unregistered.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, &mut lc, &mut mr, &mut ov, &can, None);
    let resp = extract_response(&out);
    assert_eq!(resp["id"], "req-typedef-str-1");
    assert_eq!(resp["result"], json!(null));
}

#[test]
fn test_type_definition_preserves_large_numeric_id() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 9999999,
        "method": "textDocument/typeDefinition",
        "params": {
            "textDocument": { "uri": "file:///tmp/unregistered.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, &mut lc, &mut mr, &mut ov, &can, None);
    let resp = extract_response(&out);
    assert_eq!(resp["id"], 9999999);
    assert_eq!(resp["result"], json!(null));
}

#[test]
fn test_type_definition_whitespace_returns_null() {
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let req = make_pos_req("file:///tmp/empty.rs", 0, 0);
    let res = handle_type_definition(&req, None, &overlay_mgr, &mut conv);
    assert_eq!(res, json!(null));
}

#[test]
fn test_type_definition_unregistered_file_returns_null() {
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let req = make_pos_req("file:///not/indexed.rs", 5, 10);
    let res = handle_type_definition(&req, None, &overlay_mgr, &mut conv);
    assert_eq!(res, json!(null));
}

#[test]
fn test_type_definition_empty_workspace_returns_null() {
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let req = make_pos_req("file:///test.rs", 1, 1);
    let res = handle_type_definition(&req, None, &overlay_mgr, &mut conv);
    assert_eq!(res, json!(null));
}

#[test]
fn test_type_definition_out_of_bounds_line_returns_null() {
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let req = make_pos_req("file:///test.rs", 9999, 0);
    let res = handle_type_definition(&req, None, &overlay_mgr, &mut conv);
    assert_eq!(res, json!(null));
}

#[test]
fn test_type_definition_out_of_bounds_col_returns_null() {
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let req = make_pos_req("file:///test.rs", 0, 9999);
    let res = handle_type_definition(&req, None, &overlay_mgr, &mut conv);
    assert_eq!(res, json!(null));
}

#[test]
fn test_type_definition_primitive_type_without_def_returns_null() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_t6_prim", "0.1.0").unwrap();
    ws.add_source_file("pkg_t6_prim", "src/lib.rs", "pub fn f() { let x = 42; }\n").unwrap();
    // primitive i32 has no definition symbol in workspace
    ws.add_type_at("pkg_t6_prim", "src/lib.rs", 1, 17, 1, 18, "i32").unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_t6_prim", "src/lib.rs").unwrap();

    let req = make_pos_req(&uri, 0, 17);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res, json!(null));
}
