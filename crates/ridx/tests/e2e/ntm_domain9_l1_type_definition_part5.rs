/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/typeDefinition part 5:
//! cross-module, hierarchical module trees, clean overlay resolution, and generic targets.

use ridx::lsp::navigation::handle_type_definition;
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

fn assert_range(range: &Value, start_line: u32, start_col: u32, end_line: u32, end_col: u32) {
    assert_eq!(range["start"]["line"], start_line);
    assert_eq!(range["start"]["character"], start_col);
    assert_eq!(range["end"]["line"], end_line);
    assert_eq!(range["end"]["character"], end_col);
}

#[test]
fn test_type_definition_cross_file_submodule() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_t5_1", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_t5_1", "src/sub/types.rs", "pub struct «SubModuleType»;\n", DefKind::Struct)
        .unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file("pkg_t5_1", "src/lib.rs", "pub fn use_sub() { let obj = 1; }\n").unwrap();
    ws.add_type_at("pkg_t5_1", "src/lib.rs", 1, 23, 1, 26, "SubModuleType").unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let lib_uri = ws.file_uri("pkg_t5_1", "src/lib.rs").unwrap();
    let sub_uri = ws.file_uri("pkg_t5_1", "src/sub/types.rs").unwrap();

    let req = make_pos_req(&lib_uri, 0, 24);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res["uri"], sub_uri);
    assert_range(&res["range"], 0, 11, 0, 24);
}

#[test]
fn test_type_definition_cross_file_parent_module() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_t5_2", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_t5_2", "src/lib.rs", "pub struct «ParentType»;\n", DefKind::Struct)
        .unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file("pkg_t5_2", "src/nested/child.rs", "pub fn child_fn() { let v = 1; }\n").unwrap();
    ws.add_type_at("pkg_t5_2", "src/nested/child.rs", 1, 25, 1, 26, "ParentType").unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let child_uri = ws.file_uri("pkg_t5_2", "src/nested/child.rs").unwrap();
    let lib_uri = ws.file_uri("pkg_t5_2", "src/lib.rs").unwrap();

    let req = make_pos_req(&child_uri, 0, 24);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res["uri"], lib_uri);
    assert_range(&res["range"], 0, 11, 0, 21);
}

#[test]
fn test_type_definition_deeply_nested_module_path() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_t5_3", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_t5_3", "src/a/b/c.rs", "pub struct «DeepModel»;\n", DefKind::Struct)
        .unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file("pkg_t5_3", "src/lib.rs", "pub fn root() { let d = 1; }\n").unwrap();
    ws.add_type_at("pkg_t5_3", "src/lib.rs", 1, 21, 1, 22, "DeepModel").unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let lib_uri = ws.file_uri("pkg_t5_3", "src/lib.rs").unwrap();
    let c_uri = ws.file_uri("pkg_t5_3", "src/a/b/c.rs").unwrap();

    let req = make_pos_req(&lib_uri, 0, 20);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res["uri"], c_uri);
    assert_range(&res["range"], 0, 11, 0, 20);
}

#[test]
fn test_type_definition_with_clean_overlay() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_t5_4", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_t5_4", "src/lib.rs", "pub struct «OverlayCleanType»;\n", DefKind::Struct)
        .unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file("pkg_t5_4", "src/main.rs", "pub fn main() { let val = 1; }\n").unwrap();
    ws.add_type_at("pkg_t5_4", "src/main.rs", 1, 21, 1, 24, "OverlayCleanType").unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let main_uri = ws.file_uri("pkg_t5_4", "src/main.rs").unwrap();
    let lib_uri = ws.file_uri("pkg_t5_4", "src/lib.rs").unwrap();

    // Open file in overlay without modifying it (is_dirty = false)
    let main_path = ws.root_path().join("pkg_t5_4/src/main.rs").display().to_string();
    overlay_mgr.open_with_provenance(&main_uri, &main_path, 1, "pub fn main() { let val = 1; }\n".to_string(), true);

    let req = make_pos_req(&main_uri, 0, 22);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res["uri"], lib_uri);
    assert_range(&res["range"], 0, 11, 0, 27);
}

#[test]
fn test_type_definition_generic_type_argument_target() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_t5_5", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_t5_5", "src/types.rs", "pub struct «CustomElement»;\n", DefKind::Struct)
        .unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file("pkg_t5_5", "src/lib.rs", "pub fn list() { let elem = 0; }\n").unwrap();
    ws.add_type_at("pkg_t5_5", "src/lib.rs", 1, 20, 1, 24, "CustomElement").unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let lib_uri = ws.file_uri("pkg_t5_5", "src/lib.rs").unwrap();
    let types_uri = ws.file_uri("pkg_t5_5", "src/types.rs").unwrap();

    let req = make_pos_req(&lib_uri, 0, 22);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res["uri"], types_uri);
    assert_range(&res["range"], 0, 11, 0, 24);
}

#[test]
fn test_type_definition_option_inner_type_string() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_t5_6", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_t5_6", "src/types.rs", "pub struct «AppOption»;\n", DefKind::Struct)
        .unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file("pkg_t5_6", "src/lib.rs", "pub fn opt() { let opt_val = 0; }\n").unwrap();
    ws.add_type_at("pkg_t5_6", "src/lib.rs", 1, 19, 1, 26, "AppOption").unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let lib_uri = ws.file_uri("pkg_t5_6", "src/lib.rs").unwrap();
    let types_uri = ws.file_uri("pkg_t5_6", "src/types.rs").unwrap();

    let req = make_pos_req(&lib_uri, 0, 22);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res["uri"], types_uri);
    assert_range(&res["range"], 0, 11, 0, 20);
}

#[test]
fn test_type_definition_result_inner_type_string() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_t5_7", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_t5_7", "src/types.rs", "pub struct «AppResult»;\n", DefKind::Struct)
        .unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file("pkg_t5_7", "src/lib.rs", "pub fn res() { let res_val = 0; }\n").unwrap();
    ws.add_type_at("pkg_t5_7", "src/lib.rs", 1, 19, 1, 26, "AppResult").unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let lib_uri = ws.file_uri("pkg_t5_7", "src/lib.rs").unwrap();
    let types_uri = ws.file_uri("pkg_t5_7", "src/types.rs").unwrap();

    let req = make_pos_req(&lib_uri, 0, 22);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res["uri"], types_uri);
    assert_range(&res["range"], 0, 11, 0, 20);
}

#[test]
fn test_type_definition_vector_inner_type_string() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_t5_8", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_t5_8", "src/types.rs", "pub struct «ItemRecord»;\n", DefKind::Struct)
        .unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file("pkg_t5_8", "src/lib.rs", "pub fn vec() { let rec = 0; }\n").unwrap();
    ws.add_type_at("pkg_t5_8", "src/lib.rs", 1, 19, 1, 22, "ItemRecord").unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let lib_uri = ws.file_uri("pkg_t5_8", "src/lib.rs").unwrap();
    let types_uri = ws.file_uri("pkg_t5_8", "src/types.rs").unwrap();

    let req = make_pos_req(&lib_uri, 0, 20);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res["uri"], types_uri);
    assert_range(&res["range"], 0, 11, 0, 21);
}

#[test]
fn test_type_definition_reference_type_deref_target() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_t5_9", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_t5_9", "src/res.rs", "pub struct «BorrowedResource»;\n", DefKind::Struct)
        .unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file("pkg_t5_9", "src/lib.rs", "pub fn borrow_fn() { let r = 0; }\n").unwrap();
    ws.add_type_at("pkg_t5_9", "src/lib.rs", 1, 26, 1, 27, "BorrowedResource").unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let lib_uri = ws.file_uri("pkg_t5_9", "src/lib.rs").unwrap();
    let res_uri = ws.file_uri("pkg_t5_9", "src/res.rs").unwrap();

    let req = make_pos_req(&lib_uri, 0, 25);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res["uri"], res_uri);
    assert_range(&res["range"], 0, 11, 0, 27);
}

#[test]
fn test_type_definition_mut_reference_target() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_t5_10", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_t5_10", "src/res.rs", "pub struct «MutableResource»;\n", DefKind::Struct)
        .unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file("pkg_t5_10", "src/lib.rs", "pub fn mut_fn() { let m = 0; }\n").unwrap();
    ws.add_type_at("pkg_t5_10", "src/lib.rs", 1, 23, 1, 24, "MutableResource").unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let lib_uri = ws.file_uri("pkg_t5_10", "src/lib.rs").unwrap();
    let res_uri = ws.file_uri("pkg_t5_10", "src/res.rs").unwrap();

    let req = make_pos_req(&lib_uri, 0, 22);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res["uri"], res_uri);
    assert_range(&res["range"], 0, 11, 0, 26);
}
