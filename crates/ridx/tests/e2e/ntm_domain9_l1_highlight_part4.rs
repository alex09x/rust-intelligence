/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/documentHighlight part 4:
//! compiled workspace symbol highlights for structs, enums, functions, traits, and consts.

use ridx::lsp::navigation::handle_document_highlight;
use ridx::lsp::protocol::PositionConverter;
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

#[test]
fn test_document_highlight_compiled_struct_def_and_two_refs() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_struct", "0.1.0").unwrap();

    let (_, ids) = ws
        .add_source_file_with_defs(
            "pkg_struct",
            "src/lib.rs",
            "pub struct «TargetRecord»;\n",
            DefKind::Struct,
        )
        .unwrap();

    let refs_code = "pub struct TargetRecord;\npub fn use_it() {\n    let _ = «TargetRecord»;\n    let _ = «TargetRecord»;\n}\n";
    ws.add_refs_to_def("pkg_struct", "src/lib.rs", refs_code, ids[0])
        .unwrap();

    let shard = ws.build_crate_index("pkg_struct").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let uri = ws.file_uri("pkg_struct", "src/lib.rs").unwrap();

    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let req = make_pos_req(&uri, 0, 12);
    let res = handle_document_highlight(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["kind"], 3);
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[1]["kind"], 1);
    assert_eq!(items[1]["range"]["start"]["line"], 2);
    assert_eq!(items[2]["kind"], 1);
    assert_eq!(items[2]["range"]["start"]["line"], 3);
}

#[test]
fn test_document_highlight_compiled_enum_def_and_variant_refs() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_enum", "0.1.0").unwrap();

    let (_, ids) = ws
        .add_source_file_with_defs(
            "pkg_enum",
            "src/lib.rs",
            "pub enum «Status» {\n    Active,\n    Inactive,\n}\n",
            DefKind::Enum,
        )
        .unwrap();

    let refs_code = "pub enum Status {\n    Active,\n    Inactive,\n}\npub fn check(s: «Status») {\n    let _: «Status» = s;\n}\n";
    ws.add_refs_to_def("pkg_enum", "src/lib.rs", refs_code, ids[0])
        .unwrap();

    let shard = ws.build_crate_index("pkg_enum").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let uri = ws.file_uri("pkg_enum", "src/lib.rs").unwrap();

    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let req = make_pos_req(&uri, 0, 9);
    let res = handle_document_highlight(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["kind"], 3);
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[1]["kind"], 1);
    assert_eq!(items[1]["range"]["start"]["line"], 4);
    assert_eq!(items[2]["kind"], 1);
    assert_eq!(items[2]["range"]["start"]["line"], 5);
}

#[test]
fn test_document_highlight_compiled_fn_def_and_call_site() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_fn", "0.1.0").unwrap();

    let (_, ids) = ws
        .add_source_file_with_defs(
            "pkg_fn",
            "src/lib.rs",
            "pub fn «calculate»() -> i32 { 42 }\n",
            DefKind::Fn,
        )
        .unwrap();

    let refs_code = "pub fn calculate() -> i32 { 42 }\npub fn caller() {\n    let _ = «calculate»();\n}\n";
    ws.add_refs_to_def("pkg_fn", "src/lib.rs", refs_code, ids[0])
        .unwrap();

    let shard = ws.build_crate_index("pkg_fn").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let uri = ws.file_uri("pkg_fn", "src/lib.rs").unwrap();

    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let req = make_pos_req(&uri, 0, 7);
    let res = handle_document_highlight(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["kind"], 3);
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[1]["kind"], 1);
    assert_eq!(items[1]["range"]["start"]["line"], 2);
}

#[test]
fn test_document_highlight_compiled_trait_def_and_impl() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_trait", "0.1.0").unwrap();

    let (_, ids) = ws
        .add_source_file_with_defs(
            "pkg_trait",
            "src/lib.rs",
            "pub trait «Worker» {\n    fn do_work(&self);\n}\n",
            DefKind::Trait,
        )
        .unwrap();

    let refs_code = "pub trait Worker {\n    fn do_work(&self);\n}\npub struct Job;\nimpl «Worker» for Job {\n    fn do_work(&self) {}\n}\n";
    ws.add_refs_to_def("pkg_trait", "src/lib.rs", refs_code, ids[0])
        .unwrap();

    let shard = ws.build_crate_index("pkg_trait").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let uri = ws.file_uri("pkg_trait", "src/lib.rs").unwrap();

    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let req = make_pos_req(&uri, 0, 10);
    let res = handle_document_highlight(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["kind"], 3);
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[1]["kind"], 1);
    assert_eq!(items[1]["range"]["start"]["line"], 4);
}

#[test]
fn test_document_highlight_compiled_const_def_and_usage() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_const", "0.1.0").unwrap();

    let (_, ids) = ws
        .add_source_file_with_defs(
            "pkg_const",
            "src/lib.rs",
            "pub const «MAX_LIMIT»: usize = 100;\n",
            DefKind::Const,
        )
        .unwrap();

    let refs_code = "pub const MAX_LIMIT: usize = 100;\npub fn limit_check(val: usize) -> bool {\n    val < «MAX_LIMIT»\n}\n";
    ws.add_refs_to_def("pkg_const", "src/lib.rs", refs_code, ids[0])
        .unwrap();

    let shard = ws.build_crate_index("pkg_const").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let uri = ws.file_uri("pkg_const", "src/lib.rs").unwrap();

    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let req = make_pos_req(&uri, 0, 11);
    let res = handle_document_highlight(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["kind"], 3);
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[1]["kind"], 1);
    assert_eq!(items[1]["range"]["start"]["line"], 2);
}

#[test]
fn test_document_highlight_clean_overlay_with_compiled_index() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_clean_ov", "0.1.0").unwrap();

    let (_, ids) = ws
        .add_source_file_with_defs(
            "pkg_clean_ov",
            "src/lib.rs",
            "pub struct «CleanTarget»;\n",
            DefKind::Struct,
        )
        .unwrap();

    let refs_code = "pub struct CleanTarget;\npub fn invoke() {\n    let _ = «CleanTarget»;\n}\n";
    ws.add_refs_to_def("pkg_clean_ov", "src/lib.rs", refs_code, ids[0])
        .unwrap();

    let shard = ws.build_crate_index("pkg_clean_ov").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let file_path = ws.file_path("pkg_clean_ov", "src/lib.rs").unwrap();
    let file_str = file_path.to_str().unwrap();
    let uri = ws.file_uri("pkg_clean_ov", "src/lib.rs").unwrap();

    let clean_code = std::fs::read_to_string(&file_path).unwrap();
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    overlay_mgr.open_document(&uri, file_str, 1, clean_code);

    let req = make_pos_req(&uri, 0, 12);
    let res = handle_document_highlight(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["kind"], 3);
    assert_eq!(items[1]["kind"], 1);
}
