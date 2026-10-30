/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Synthetic hermetic workspace integration tests for Domain 9: Definition & Declaration.
//! Covers textDocument/definition and textDocument/declaration across single-file,
//! cross-file, type aliases, overlay shifts, and error boundaries.

use ridx::lsp::navigation::{handle_declaration, handle_definition};
use ridx::lsp::protocol::PositionConverter;
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

#[test]
fn test_l1_definition_in_same_file() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_a", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_a", "src/lib.rs", "pub struct «TargetAlpha»;\n", DefKind::Struct).unwrap();
    ws.add_refs_to_def("pkg_a", "src/lib.rs", "pub struct TargetAlpha;\npub fn invoke() { let _ = «TargetAlpha»; }\n", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_a").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_a", "src/lib.rs").unwrap();

    let req = make_def_req(&uri, 1, 26);
    let res = handle_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert!(!res.is_null(), "Must find definition in same file");
    assert_eq!(res["uri"], uri);
    assert_eq!(res["range"]["start"]["line"], 0);
    assert_eq!(res["range"]["start"]["character"], 11);
    assert_eq!(res["range"]["end"]["character"], 22);
}

#[test]
fn test_l1_declaration_matches_definition_same_file() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_b", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_b", "src/lib.rs", "pub fn «service_handler»() -> i32 { 100 }\n", DefKind::Fn).unwrap();
    ws.add_refs_to_def("pkg_b", "src/lib.rs", "pub fn service_handler() -> i32 { 100 }\npub fn caller() { let _ = «service_handler»(); }\n", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_b").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_b", "src/lib.rs").unwrap();

    let def_req = make_def_req(&uri, 1, 26);
    let def_res = handle_definition(&def_req, Some(&index), &overlay_mgr, &mut conv);
    let decl_res = handle_declaration(&def_req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(def_res, decl_res, "Declaration must match definition result");
}

#[test]
fn test_l1_definition_cross_file_same_crate() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_c", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_c", "src/models.rs", "pub struct «SharedModel»;\n", DefKind::Struct).unwrap();
    ws.add_refs_to_def("pkg_c", "src/lib.rs", "mod models;\nuse models::SharedModel;\npub fn run() { let _ = «SharedModel»; }\n", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_c").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let lib_uri = ws.file_uri("pkg_c", "src/lib.rs").unwrap();
    let models_uri = ws.file_uri("pkg_c", "src/models.rs").unwrap();

    let req = make_def_req(&lib_uri, 2, 23);
    let res = handle_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert!(!res.is_null());
    assert_eq!(res["uri"], models_uri);
    assert_eq!(res["range"]["start"]["line"], 0);
    assert_eq!(res["range"]["start"]["character"], 11);
}

#[test]
fn test_l1_declaration_cross_file_same_crate() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_d", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_d", "src/utils.rs", "pub fn «calculate_sum»(a: i32, b: i32) -> i32 { a + b }\n", DefKind::Fn).unwrap();
    ws.add_refs_to_def("pkg_d", "src/lib.rs", "mod utils;\npub fn main_entry() { let _ = utils::«calculate_sum»(1, 2); }\n", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_d").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let lib_uri = ws.file_uri("pkg_d", "src/lib.rs").unwrap();
    let utils_uri = ws.file_uri("pkg_d", "src/utils.rs").unwrap();

    let req = make_def_req(&lib_uri, 1, 37);
    let decl_res = handle_declaration(&req, Some(&index), &overlay_mgr, &mut conv);
    assert!(!decl_res.is_null());
    assert_eq!(decl_res["uri"], utils_uri);
}

#[test]
fn test_l1_definition_cursor_on_whitespace_returns_null() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_e", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_e", "src/lib.rs", "pub struct «SomeStruct»;\n", DefKind::Struct).unwrap();
    let shard = ws.build_crate_index("pkg_e").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_e", "src/lib.rs").unwrap();

    let req = make_def_req(&uri, 0, 3);
    let res = handle_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert!(res.is_null(), "Cursor on whitespace must return null");
}

#[test]
fn test_l1_definition_out_of_bounds_returns_null() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_f", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_f", "src/lib.rs", "pub struct «Item»;\n", DefKind::Struct).unwrap();
    let shard = ws.build_crate_index("pkg_f").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_f", "src/lib.rs").unwrap();

    let res_line = handle_definition(&make_def_req(&uri, 999, 0), Some(&index), &overlay_mgr, &mut conv);
    assert!(res_line.is_null());

    let res_col = handle_definition(&make_def_req(&uri, 0, 9999), Some(&index), &overlay_mgr, &mut conv);
    assert!(res_col.is_null());
}

#[test]
fn test_l1_definition_unregistered_file_uri_returns_null() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_g", "0.1.0").unwrap();
    let shard = ws.build_crate_index("pkg_g").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let req = make_def_req("file:///non/existent/path/missing.rs", 0, 0);
    let res = handle_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert!(res.is_null(), "Missing file must return null");
}

#[test]
fn test_l1_definition_empty_params_returns_null_safe() {
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let empty_req = json!({});
    let res = handle_definition(&empty_req, None, &overlay_mgr, &mut conv);
    assert!(res.is_null(), "Malformed empty request must safely return null");
}

#[test]
fn test_l1_definition_type_alias_synthetic() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_k", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_k", "src/lib.rs", "pub type «CustomAlias» = i32;\n", DefKind::TyAlias).unwrap();
    ws.add_refs_to_def("pkg_k", "src/lib.rs", "pub type CustomAlias = i32;\npub fn run(x: «CustomAlias») -> i32 { x }\n", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_k").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_k", "src/lib.rs").unwrap();

    let req = make_def_req(&uri, 1, 14);
    let res = handle_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert!(!res.is_null());
    assert_eq!(res["range"]["start"]["line"], 0);
    assert_eq!(res["range"]["start"]["character"], 9);
}

#[test]
fn test_l1_overlay_preserves_navigation() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_l", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_l", "src/lib.rs", "pub struct «StateNode»;\n", DefKind::Struct).unwrap();
    ws.add_refs_to_def("pkg_l", "src/lib.rs", "pub struct StateNode;\npub fn run() { let _ = «StateNode»; }\n", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_l").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let uri = ws.file_uri("pkg_l", "src/lib.rs").unwrap();
    let file_path = ws.file_path("pkg_l", "src/lib.rs").unwrap();

    let file_str = file_path.to_str().unwrap();

    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let initial_content = ws.file_content("pkg_l", "src/lib.rs").unwrap();
    overlay_mgr.open_document(&uri, file_str, 1, initial_content.to_string());

    // 1. Initial clean overlay: query at line 1 (0-based), col 23
    let req = make_def_req(&uri, 1, 23);
    let res = handle_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert!(!res.is_null());
    assert_eq!(res["range"]["start"]["line"], 0);

    // 2. Incremental change: insert comment line at line 1 col 0, shifting usage to line 2
    let edits = json!([
        {
            "range": {
                "start": { "line": 1, "character": 0 },
                "end": { "line": 1, "character": 0 }
            },
            "rangeLength": 0,
            "text": "// comment line\n"
        }
    ]);
    overlay_mgr.apply_content_changes(file_str, 2, edits.as_array().unwrap());

    // Query at new shifted position on line 2, col 23 -> must resolve to the same target definition at line 0
    let shifted_req = make_def_req(&uri, 2, 23);
    let shifted_res = handle_definition(&shifted_req, Some(&index), &overlay_mgr, &mut conv);
    assert!(!shifted_res.is_null(), "Shifted position in dirty overlay must resolve to compiled definition");
    assert_eq!(shifted_res["range"]["start"]["line"], 0);
}
