/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::navigation::{handle_declaration, handle_definition};
use ridx::lsp::protocol::{LspMessage, PositionConverter};
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use ridx_test_support::fixture::SyntheticWorkspaceFixture;
use serde_json::{json, Value};

fn make_def_req(uri: &str, line: u32, col: u32) -> Value {
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

fn setup_initialized_server() -> (MultiRootWorkspace, OverlayManager, LifecycleManager, Arc<Mutex<HashSet<serde_json::Value>>>) {
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
fn test_definition_struct_type_in_same_file() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_part3_a", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_part3_a", "src/lib.rs", "pub struct «DataRecord»;\n", DefKind::Struct).unwrap();
    ws.add_refs_to_def("pkg_part3_a", "src/lib.rs", "pub struct DataRecord;\npub fn run() { let _ = «DataRecord»; }\n", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_part3_a").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_part3_a", "src/lib.rs").unwrap();

    let req = make_def_req(&uri, 1, 23);
    let res = handle_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res["uri"], uri);
    assert_range(&res["range"], 0, 11, 0, 21);
}

#[test]
fn test_definition_function_symbol_in_same_file() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_part3_b", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_part3_b", "src/lib.rs", "pub fn «compute_sum»() -> i32 { 42 }\n", DefKind::Fn).unwrap();
    ws.add_refs_to_def("pkg_part3_b", "src/lib.rs", "pub fn compute_sum() -> i32 { 42 }\npub fn main() { let _ = «compute_sum»(); }\n", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_part3_b").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_part3_b", "src/lib.rs").unwrap();

    let req = make_def_req(&uri, 1, 25);
    let res = handle_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res["uri"], uri);
    assert_range(&res["range"], 0, 7, 0, 18);
}

#[test]
fn test_definition_enum_type_in_same_file() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_part3_c", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_part3_c", "src/lib.rs", "pub enum «ColorMode» { Dark, Light }\n", DefKind::Enum).unwrap();
    ws.add_refs_to_def("pkg_part3_c", "src/lib.rs", "pub enum ColorMode { Dark, Light }\npub fn get_mode() -> «ColorMode» { todo!() }\n", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_part3_c").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_part3_c", "src/lib.rs").unwrap();

    let req = make_def_req(&uri, 1, 22);
    let res = handle_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res["uri"], uri);
    assert_range(&res["range"], 0, 9, 0, 18);
}

#[test]
fn test_definition_const_item_in_same_file() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_part3_d", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_part3_d", "src/lib.rs", "pub const «BUFFER_CAP»: usize = 1024;\n", DefKind::Const).unwrap();
    ws.add_refs_to_def("pkg_part3_d", "src/lib.rs", "pub const BUFFER_CAP: usize = 1024;\npub fn init() { let _ = «BUFFER_CAP»; }\n", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_part3_d").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_part3_d", "src/lib.rs").unwrap();

    let req = make_def_req(&uri, 1, 25);
    let res = handle_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res["uri"], uri);
    assert_range(&res["range"], 0, 10, 0, 20);
}

#[test]
fn test_definition_preserves_numeric_request_id() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 77777,
        "method": "textDocument/definition",
        "params": {
            "textDocument": { "uri": "file:///tmp/unregistered.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, &mut lc, &mut mr, &mut ov, &can, None);
    let resp = extract_response(&out);
    assert_eq!(resp["id"], 77777);
    assert_eq!(resp["result"], json!(null));
}

#[test]
fn test_definition_preserves_string_request_id() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": "req-def-id-42",
        "method": "textDocument/definition",
        "params": {
            "textDocument": { "uri": "file:///tmp/unregistered.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, &mut lc, &mut mr, &mut ov, &can, None);
    let resp = extract_response(&out);
    assert_eq!(resp["id"], "req-def-id-42");
    assert_eq!(resp["result"], json!(null));
}

#[test]
fn test_definition_delegating_declaration_parity() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_part3_e", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_part3_e", "src/lib.rs", "pub struct «SharedType»;\n", DefKind::Struct).unwrap();
    ws.add_refs_to_def("pkg_part3_e", "src/lib.rs", "pub struct SharedType;\npub fn run() { let _ = «SharedType»; }\n", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_part3_e").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv_def = PositionConverter::new();
    let mut conv_decl = PositionConverter::new();
    let uri = ws.file_uri("pkg_part3_e", "src/lib.rs").unwrap();

    let req = make_def_req(&uri, 1, 23);
    let res_def = handle_definition(&req, Some(&index), &overlay_mgr, &mut conv_def);
    let res_decl = handle_declaration(&req, Some(&index), &overlay_mgr, &mut conv_decl);
    assert_eq!(res_decl, res_def);
    assert_eq!(res_decl["uri"], uri);
}

#[test]
fn test_definition_whitespace_position_returns_null() {
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let req = make_def_req("file:///tmp/some_file.rs", 10, 0);
    let res = handle_definition(&req, None, &overlay_mgr, &mut conv);
    assert_eq!(res, json!(null));
}
