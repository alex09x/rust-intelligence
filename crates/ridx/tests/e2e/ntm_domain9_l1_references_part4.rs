/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/references part 4:
//! includeDeclaration flag verification, callsite resolution, and item kinds.

use ridx::lsp::navigation::references::handle_references;
use ridx::lsp::protocol::PositionConverter;
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

#[test]
fn test_references_include_declaration_true_includes_def_site() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_r4_1", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_r4_1", "src/lib.rs", "pub struct «TargetDecl»;\n", DefKind::Struct).unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub struct TargetDecl;\npub fn run() { let _ = «TargetDecl»; }\n";
    ws.add_refs_to_def("pkg_r4_1", "src/lib.rs", code, ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_r4_1").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_r4_1", "src/lib.rs").unwrap();

    let req = make_ref_req(&uri, 0, 11, true);
    let res = handle_references(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 2);
}

#[test]
fn test_references_include_declaration_false_excludes_def_site() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_r4_2", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_r4_2", "src/lib.rs", "pub struct «TargetNoDecl»;\n", DefKind::Struct).unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub struct TargetNoDecl;\npub fn run() { let _ = «TargetNoDecl»; }\n";
    ws.add_refs_to_def("pkg_r4_2", "src/lib.rs", code, ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_r4_2").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_r4_2", "src/lib.rs").unwrap();

    let req = make_ref_req(&uri, 0, 11, false);
    let res = handle_references(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["range"]["start"]["line"], 1);
}

#[test]
fn test_references_multiple_callsites_in_same_function() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_r4_3", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_r4_3", "src/lib.rs", "pub struct «MultiCall»;\n", DefKind::Struct).unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub struct MultiCall;\npub fn run() {\n    let _a = «MultiCall»;\n    let _b = «MultiCall»;\n}\n";
    ws.add_refs_to_def("pkg_r4_3", "src/lib.rs", code, ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_r4_3").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_r4_3", "src/lib.rs").unwrap();

    let req = make_ref_req(&uri, 0, 11, false);
    let res = handle_references(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 2);
}

#[test]
fn test_references_callsites_across_multiple_functions() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_r4_4", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_r4_4", "src/lib.rs", "pub struct «CrossFnTarget»;\n", DefKind::Struct).unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub struct CrossFnTarget;\npub fn f1() { let _ = «CrossFnTarget»; }\npub fn f2() { let _ = «CrossFnTarget»; }\n";
    ws.add_refs_to_def("pkg_r4_4", "src/lib.rs", code, ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_r4_4").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_r4_4", "src/lib.rs").unwrap();

    let req = make_ref_req(&uri, 0, 11, false);
    let res = handle_references(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 2);
}

#[test]
fn test_references_function_symbol_references() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_r4_5", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_r4_5", "src/lib.rs", "pub fn «helper_tool»() {}\n", DefKind::Fn).unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub fn helper_tool() {}\npub fn main() { «helper_tool»(); }\n";
    ws.add_refs_to_def("pkg_r4_5", "src/lib.rs", code, ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_r4_5").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_r4_5", "src/lib.rs").unwrap();

    let req = make_ref_req(&uri, 0, 7, true);
    let res = handle_references(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 2);
}

#[test]
fn test_references_enum_symbol_references() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_r4_6", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_r4_6", "src/lib.rs", "pub enum «Direction» { Up, Down }\n", DefKind::Enum).unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub enum Direction { Up, Down }\npub fn step() -> «Direction» { todo!() }\n";
    ws.add_refs_to_def("pkg_r4_6", "src/lib.rs", code, ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_r4_6").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_r4_6", "src/lib.rs").unwrap();

    let req = make_ref_req(&uri, 0, 9, false);
    let res = handle_references(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["range"]["start"]["line"], 1);
}

#[test]
fn test_references_const_symbol_references() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_r4_7", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_r4_7", "src/lib.rs", "pub const «MAX_SIZE»: usize = 512;\n", DefKind::Const).unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub const MAX_SIZE: usize = 512;\npub fn check() { let _ = «MAX_SIZE»; }\n";
    ws.add_refs_to_def("pkg_r4_7", "src/lib.rs", code, ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_r4_7").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_r4_7", "src/lib.rs").unwrap();

    let req = make_ref_req(&uri, 0, 10, true);
    let res = handle_references(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 2);
}

#[test]
fn test_references_query_from_reference_site_finds_all() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_r4_8", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_r4_8", "src/lib.rs", "pub struct «Bidirectional»;\n", DefKind::Struct).unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub struct Bidirectional;\npub fn a() { let _ = «Bidirectional»; }\npub fn b() { let _ = «Bidirectional»; }\n";
    ws.add_refs_to_def("pkg_r4_8", "src/lib.rs", code, ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_r4_8").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_r4_8", "src/lib.rs").unwrap();

    // Query from line 1 (the first callsite) instead of line 0
    let req = make_ref_req(&uri, 1, 23, true);
    let res = handle_references(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 3);
}

#[test]
fn test_references_query_from_definition_site_finds_all() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_r4_9", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_r4_9", "src/lib.rs", "pub struct «DefSiteQuery»;\n", DefKind::Struct).unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub struct DefSiteQuery;\npub fn run() { let _ = «DefSiteQuery»; }\n";
    ws.add_refs_to_def("pkg_r4_9", "src/lib.rs", code, ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_r4_9").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_r4_9", "src/lib.rs").unwrap();

    let req = make_ref_req(&uri, 0, 11, true);
    let res = handle_references(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 2);
}

#[test]
fn test_references_unused_symbol_with_include_declaration_false() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_r4_10", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_r4_10", "src/lib.rs", "pub struct «LonelyStruct»;\n", DefKind::Struct).unwrap();
    assert_eq!(ids.len(), 1);

    let shard = ws.build_crate_index("pkg_r4_10").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_r4_10", "src/lib.rs").unwrap();

    let req = make_ref_req(&uri, 0, 11, false);
    let res = handle_references(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 0);
}
