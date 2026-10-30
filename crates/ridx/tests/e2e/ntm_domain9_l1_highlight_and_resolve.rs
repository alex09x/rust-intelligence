/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/documentHighlight and workspaceSymbol/resolve:
//! highlight kinds contract, parameter highlights, macro usages, and resolve containerName semantics.

use ridx::lsp::navigation::handle_document_highlight;
use ridx::lsp::protocol::PositionConverter;
use ridx::lsp::symbols::handle_workspace_symbol_resolve;
use ridx::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition, WorkspaceIndex};
use ridx_test_support::fixture::SyntheticWorkspaceFixture;
use serde_json::{json, Value};
use std::fs;

fn make_pos_req(uri: &str, line: u32, col: u32) -> Value {
    json!({
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": col }
        }
    })
}

#[test]
fn test_l1_document_highlight_kinds_contract_decl_and_usage() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/kinds.rs";
    let file = "/workspace/src/kinds.rs";
    let code = "let mut value = 10;\nlet next = value + 1;\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_pos_req(uri, 0, 8);
    let res = handle_document_highlight(&req, None, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("highlight list");
    assert_eq!(items.len(), 2);

    // Declaration has kind 3 (Write), usage has kind 1 (Text)
    assert_eq!(items[0]["kind"], 3);
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[0]["range"]["start"]["character"], 8);
    assert_eq!(items[0]["range"]["end"]["character"], 13);

    assert_eq!(items[1]["kind"], 1);
    assert_eq!(items[1]["range"]["start"]["line"], 1);
    assert_eq!(items[1]["range"]["start"]["character"], 11);
    assert_eq!(items[1]["range"]["end"]["character"], 16);
}

#[test]
fn test_l1_document_highlight_let_mut_reassignment() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/mut_reassign.rs";
    let file = "/workspace/src/mut_reassign.rs";
    let code = "let mut x = 1;\nx = 2;\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_pos_req(uri, 0, 8);
    let res = handle_document_highlight(&req, None, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("highlight list");
    assert_eq!(items.len(), 2);

    assert_eq!(items[0]["kind"], 3); // Declaration / binding definition is Write (3)
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[0]["range"]["start"]["character"], 8);
    assert_eq!(items[0]["range"]["end"]["character"], 9);

    // Pin current RIDX behavior: reassignment reported as Text (1), not Write (3)
    assert_eq!(items[1]["kind"], 1);
    assert_eq!(items[1]["range"]["start"]["line"], 1);
    assert_eq!(items[1]["range"]["start"]["character"], 0);
    assert_eq!(items[1]["range"]["end"]["character"], 1);
}

#[test]
fn test_l1_document_highlight_function_parameter() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/params.rs";
    let file = "/workspace/src/params.rs";
    let code = "pub fn compute(factor: i32) -> i32 {\n    factor * 2\n}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    // Cursor on `factor` parameter declaration
    let req = make_pos_req(uri, 0, 15);
    let res = handle_document_highlight(&req, None, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("highlight list");
    assert_eq!(items.len(), 2);

    assert_eq!(items[0]["kind"], 3);
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[0]["range"]["start"]["character"], 15);
    assert_eq!(items[0]["range"]["end"]["character"], 21);

    assert_eq!(items[1]["kind"], 1);
    assert_eq!(items[1]["range"]["start"]["line"], 1);
    assert_eq!(items[1]["range"]["start"]["character"], 4);
    assert_eq!(items[1]["range"]["end"]["character"], 10);
}

#[test]
fn test_l1_document_highlight_symbol_in_macro_invocation() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/macro_usage.rs";
    let file = "/workspace/src/macro_usage.rs";
    let code = "let tag = 42;\nprintln!(\"tag is: {}\", tag);\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_pos_req(uri, 0, 4);
    let res = handle_document_highlight(&req, None, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("highlight list");

    // String literal "tag is: {}" at col 10 contains substring "tag",
    // but is correctly excluded from highlights (only 2 items returned: definition and argument usage)
    assert_eq!(items.len(), 2);

    assert_eq!(items[0]["kind"], 3);
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[0]["range"]["start"]["character"], 4);
    assert_eq!(items[0]["range"]["end"]["character"], 7);

    assert_eq!(items[1]["kind"], 1);
    assert_eq!(items[1]["range"]["start"]["line"], 1);
    assert_eq!(items[1]["range"]["start"]["character"], 23);
    assert_eq!(items[1]["range"]["end"]["character"], 26);
}

#[test]
fn test_l1_document_highlight_compiled_struct_field_self_access() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_field", "0.1.0").unwrap();

    let (_, ids) = ws.add_source_file_with_defs("pkg_field", "src/lib.rs", "pub struct Point { pub «x»: i32 }\n", DefKind::Field).unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_refs_to_def("pkg_field", "src/lib.rs", "pub struct Point { pub x: i32 }\nimpl Point { pub fn get_x(&self) -> i32 { self.«x» } }\n", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_field").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_field", "src/lib.rs").unwrap();

    // Query highlight on field definition at line 0, col 23
    let req = make_pos_req(&uri, 0, 23);
    let res = handle_document_highlight(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().expect("highlights");
    assert_eq!(items.len(), 2);

    assert_eq!(items[0]["kind"], 3); // Declaration
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[0]["range"]["start"]["character"], 23);
    assert_eq!(items[0]["range"]["end"]["character"], 24);

    assert_eq!(items[1]["kind"], 1); // Usage in self.x
    assert_eq!(items[1]["range"]["start"]["line"], 1);
    assert_eq!(items[1]["range"]["start"]["character"], 47);
    assert_eq!(items[1]["range"]["end"]["character"], 48);
}

#[test]
fn test_l1_workspace_symbol_resolve_nested_module_container_name() {
    let ws = SyntheticWorkspaceFixture::new().unwrap();

    let mut writer = IndexWriter::new("pkg_net", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        file: "/workspace/src/client.rs".to_string(),
        name: "Client".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 7,
        path: "pkg_net::net::http::Client".to_string(),
        signature: None,
    });
    let shard_dir = ws.root_path().join("shard");
    fs::create_dir_all(&shard_dir).unwrap();
    let shard_path = shard_dir.join("net.ridx");
    let mut f = fs::File::create(&shard_path).unwrap();
    writer.write_to(&mut f).unwrap();

    let index = WorkspaceIndex::open(&shard_path).unwrap();
    let mut conv = PositionConverter::new();
    let req = json!({
        "params": {
            "name": "Client"
        }
    });

    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&index), &mut conv);
    assert_eq!(res["name"], "Client");
    assert_eq!(res["containerName"], "http");
    assert_eq!(res["location"]["uri"], "file:///workspace/src/client.rs");
    assert_eq!(res["location"]["range"]["start"]["line"], 0);
    assert_eq!(res["location"]["range"]["start"]["character"], 0);
    assert_eq!(res["location"]["range"]["end"]["line"], 0);
    assert_eq!(res["location"]["range"]["end"]["character"], 6);
}

#[test]
fn test_l1_workspace_symbol_resolve_crate_root_container_name() {
    let ws = SyntheticWorkspaceFixture::new().unwrap();

    let mut writer = IndexWriter::new("pkg_root", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        file: "/workspace/src/server.rs".to_string(),
        name: "Server".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 7,
        path: "pkg_root::Server".to_string(),
        signature: None,
    });
    let shard_path = ws.root_path().join("root.ridx");
    let mut f = fs::File::create(&shard_path).unwrap();
    writer.write_to(&mut f).unwrap();

    let index = WorkspaceIndex::open(&shard_path).unwrap();
    let mut conv = PositionConverter::new();
    let req = json!({
        "params": {
            "name": "Server"
        }
    });

    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&index), &mut conv);
    assert_eq!(res["name"], "Server");
    assert_eq!(res["containerName"], "pkg_root");
}

#[test]
fn test_l1_workspace_symbol_resolve_bare_path_empty_container_name() {
    let ws = SyntheticWorkspaceFixture::new().unwrap();

    let mut writer = IndexWriter::new("pkg_bare", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        file: "/workspace/src/bare.rs".to_string(),
        name: "BareItem".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 9,
        path: "BareItem".to_string(),
        signature: None,
    });
    let shard_path = ws.root_path().join("bare.ridx");
    let mut f = fs::File::create(&shard_path).unwrap();
    writer.write_to(&mut f).unwrap();

    let index = WorkspaceIndex::open(&shard_path).unwrap();
    let mut conv = PositionConverter::new();
    let req = json!({
        "params": {
            "name": "BareItem"
        }
    });

    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&index), &mut conv);
    assert_eq!(res["name"], "BareItem");
    assert_eq!(res["containerName"], "");
}

#[test]
fn test_l1_workspace_symbol_resolve_preserves_custom_client_payload() {
    let ws = SyntheticWorkspaceFixture::new().unwrap();

    let mut writer = IndexWriter::new("pkg_payload", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: "/workspace/src/payload.rs".to_string(),
        name: "target_func".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 12,
        path: "pkg_payload::target_func".to_string(),
        signature: None,
    });
    let shard_path = ws.root_path().join("payload.ridx");
    let mut f = fs::File::create(&shard_path).unwrap();
    writer.write_to(&mut f).unwrap();

    let index = WorkspaceIndex::open(&shard_path).unwrap();
    let mut conv = PositionConverter::new();
    let req = json!({
        "params": {
            "name": "target_func",
            "kind": 12,
            "data": { "clientRequestId": 99 },
            "customFlag": true
        }
    });

    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&index), &mut conv);
    assert_eq!(res["name"], "target_func");
    assert_eq!(res["kind"], 12);
    assert_eq!(res["data"]["clientRequestId"], 99);
    assert_eq!(res["customFlag"], true);
    assert_eq!(res["containerName"], "pkg_payload");
    assert_eq!(res["location"]["uri"], "file:///workspace/src/payload.rs");
    assert_eq!(res["location"]["range"]["start"]["line"], 0);
    assert_eq!(res["location"]["range"]["start"]["character"], 0);
    assert_eq!(res["location"]["range"]["end"]["line"], 0);
    assert_eq!(res["location"]["range"]["end"]["character"], 11);
}

#[test]
fn test_l1_workspace_symbol_resolve_cross_shard_multi_crate() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("crate_alpha", "0.1.0").unwrap();
    ws.add_crate("crate_beta", "0.1.0").unwrap();

    let shard_a = ws.build_crate_index("crate_alpha").unwrap();
    let (_, ids_b) = ws.add_source_file_with_defs("crate_beta", "src/lib.rs", "pub struct «BetaService»;\n", DefKind::Struct).unwrap();
    assert_eq!(ids_b.len(), 1);
    let shard_b = ws.build_crate_index("crate_beta").unwrap();

    let idx_a = WorkspaceIndex::open(&shard_a).unwrap();
    let idx_b = WorkspaceIndex::open(&shard_b).unwrap();
    let mut conv = PositionConverter::new();

    let req = json!({
        "params": {
            "name": "BetaService"
        }
    });

    let workspaces = vec![&idx_a, &idx_b];
    let res = handle_workspace_symbol_resolve(&req, workspaces.into_iter(), &mut conv);
    assert_eq!(res["name"], "BetaService");
    assert_eq!(res["containerName"], "crate_beta");
    let beta_uri = ws.file_uri("crate_beta", "src/lib.rs").unwrap();
    assert_eq!(res["location"]["uri"], beta_uri);
    assert_eq!(res["location"]["range"]["start"]["line"], 0);
    assert_eq!(res["location"]["range"]["start"]["character"], 11);
    assert_eq!(res["location"]["range"]["end"]["line"], 0);
    assert_eq!(res["location"]["range"]["end"]["character"], 22);
}
