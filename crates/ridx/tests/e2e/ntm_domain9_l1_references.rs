/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Synthetic hermetic workspace integration tests for Domain 9: References.
//! Tests textDocument/references across single-file, cross-file, local bindings,
//! includeDeclaration options, and macro expansion / re-export flags.

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
fn test_l1_references_same_file_multiple_usages() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_h", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs(
            "pkg_h",
            "src/lib.rs",
            "pub struct «TargetNode»;\n",
            DefKind::Struct,
        )
        .unwrap();
    ws.add_refs_to_def(
        "pkg_h",
        "src/lib.rs",
        "pub struct TargetNode;\npub fn a() { let _ = «TargetNode»; }\npub fn b() { let _ = «TargetNode»; }\n",
        ids[0],
    ).unwrap();

    let shard = ws.build_crate_index("pkg_h").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_h", "src/lib.rs").unwrap();

    // Query on definition with includeDeclaration = false
    let req_no_decl = make_ref_req(&uri, 0, 11, false);
    let res_no_decl = handle_references(&req_no_decl, Some(&index), &overlay_mgr, &mut conv);
    let refs_arr = res_no_decl.as_array().expect("references array");
    assert_eq!(refs_arr.len(), 2, "Must return exactly 2 usage references");

    // Query with includeDeclaration = true
    let req_with_decl = make_ref_req(&uri, 0, 11, true);
    let res_with_decl = handle_references(&req_with_decl, Some(&index), &overlay_mgr, &mut conv);
    let with_decl_arr = res_with_decl.as_array().expect("references array");
    assert_eq!(
        with_decl_arr.len(),
        3,
        "Must return 2 usages plus 1 declaration"
    );
    assert_eq!(with_decl_arr[0]["range"]["start"]["line"], 0);
}

#[test]
fn test_l1_references_cross_file() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_i", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs(
            "pkg_i",
            "src/shared.rs",
            "pub struct «SharedState»;\n",
            DefKind::Struct,
        )
        .unwrap();
    ws.add_refs_to_def(
        "pkg_i",
        "src/lib.rs",
        "mod shared;\nuse shared::SharedState;\npub fn run() { let _ = «SharedState»; }\n",
        ids[0],
    )
    .unwrap();

    let shard = ws.build_crate_index("pkg_i").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let shared_uri = ws.file_uri("pkg_i", "src/shared.rs").unwrap();
    let lib_uri = ws.file_uri("pkg_i", "src/lib.rs").unwrap();

    let req = make_ref_req(&shared_uri, 0, 11, false);
    let res = handle_references(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().expect("references array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["uri"], lib_uri);
    assert_eq!(arr[0]["range"]["start"]["line"], 2);
}

#[test]
fn test_l1_references_unused_symbol_returns_empty_without_decl_and_one_with_decl() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_j", "0.1.0").unwrap();
    ws.add_source_file_with_defs(
        "pkg_j",
        "src/lib.rs",
        "pub struct «UnusedSymbol»;\n",
        DefKind::Struct,
    )
    .unwrap();

    let shard = ws.build_crate_index("pkg_j").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_j", "src/lib.rs").unwrap();

    let req_no_decl = make_ref_req(&uri, 0, 11, false);
    let res_no_decl = handle_references(&req_no_decl, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res_no_decl.as_array().unwrap().len(), 0);

    let req_decl = make_ref_req(&uri, 0, 11, true);
    let res_decl = handle_references(&req_decl, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res_decl.as_array().unwrap().len(), 1);
}

#[test]
fn test_l1_references_overlay_local_bindings() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let file = "/dummy/local_test.rs";
    let uri = "file:///dummy/local_test.rs";
    let code = "fn test_local() {\n    let val_x = 42;\n    let _ = val_x + 1;\n    let _ = val_x * 2;\n}\n";

    overlay_mgr.open_document(uri, file, 1, code.to_string());

    // 1. With includeDeclaration = true: exactly 3 references at lines 1, 2, 3
    let req_with_decl = make_ref_req(uri, 1, 8, true);
    let res_with_decl = handle_references(&req_with_decl, None, &overlay_mgr, &mut conv);
    let arr_with_decl = res_with_decl.as_array().expect("references array");
    assert_eq!(
        arr_with_decl.len(),
        3,
        "Must return exactly 3 references with declaration"
    );
    assert_eq!(arr_with_decl[0]["range"]["start"]["line"], 1);
    assert_eq!(arr_with_decl[1]["range"]["start"]["line"], 2);
    assert_eq!(arr_with_decl[2]["range"]["start"]["line"], 3);

    // 2. With includeDeclaration = false: exactly 2 usage references at lines 2, 3
    let req_no_decl = make_ref_req(uri, 1, 8, false);
    let res_no_decl = handle_references(&req_no_decl, None, &overlay_mgr, &mut conv);
    let arr_no_decl = res_no_decl.as_array().expect("references array");
    assert_eq!(
        arr_no_decl.len(),
        2,
        "Must return exactly 2 references without declaration"
    );
    assert_eq!(arr_no_decl[0]["range"]["start"]["line"], 2);
    assert_eq!(arr_no_decl[1]["range"]["start"]["line"], 3);
}

#[test]
fn test_l1_references_empty_params_safe() {
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let empty_req = json!({});
    let res = handle_references(&empty_req, None, &overlay_mgr, &mut conv);
    assert_eq!(res.as_array().unwrap().len(), 0);
}

#[test]
fn test_l1_references_unregistered_uri_returns_empty() {
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let req = make_ref_req("file:///unregistered_file.rs", 0, 0, true);
    let res = handle_references(&req, None, &overlay_mgr, &mut conv);
    assert_eq!(res.as_array().unwrap().len(), 0);
}

#[test]
fn test_l1_references_out_of_bounds_coords_returns_empty() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_m", "0.1.0").unwrap();
    ws.add_source_file("pkg_m", "src/lib.rs", "pub fn existing() {}\n")
        .unwrap();
    let shard = ws.build_crate_index("pkg_m").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_m", "src/lib.rs").unwrap();

    let req = make_ref_req(&uri, 9999, 9999, true);
    let res = handle_references(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res.as_array().unwrap().len(), 0);
}

#[test]
fn test_l1_references_on_callsite_finds_all_references() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_callsite", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs(
            "pkg_callsite",
            "src/lib.rs",
            "pub struct «TargetType»;\n",
            DefKind::Struct,
        )
        .unwrap();
    ws.add_refs_to_def(
        "pkg_callsite",
        "src/lib.rs",
        "pub struct TargetType;\npub fn foo() { let _ = «TargetType»; }\npub fn bar() { let _ = «TargetType»; }\n",
        ids[0],
    ).unwrap();

    let shard = ws.build_crate_index("pkg_callsite").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_callsite", "src/lib.rs").unwrap();

    // Query references starting from callsite in foo() (line 1, col 24 in 0-based)
    let req = make_ref_req(&uri, 1, 24, true);
    let res = handle_references(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().expect("references array");
    assert_eq!(
        arr.len(),
        3,
        "Query on callsite must find definition and all sibling usages"
    );
}

#[test]
fn test_l1_references_local_parameter_without_overlay_returns_empty() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_param", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs(
            "pkg_param",
            "src/lib.rs",
            "pub fn «compute»(param_x: i32) -> i32 {\n    param_x * 2\n}\n",
            DefKind::Fn,
        )
        .unwrap();
    assert_eq!(ids.len(), 1);

    let shard = ws.build_crate_index("pkg_param").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_param", "src/lib.rs").unwrap();

    // Verify indexed function itself can be resolved
    let fn_req = make_ref_req(&uri, 0, 8, true);
    let fn_res = handle_references(&fn_req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(
        fn_res.as_array().map(|a| a.len()),
        Some(1),
        "Querying the indexed function itself must find its definition"
    );

    // Query on `param_x` on line 0, col 16 without an open overlay.
    // Position lies inside indexed function, but unindexed parameter without overlay
    // must return empty list rather than falling through to enclosing definition.
    let req = make_ref_req(&uri, 0, 16, true);
    let res = handle_references(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().expect("references array");
    assert!(
        arr.is_empty(),
        "Querying references on local parameter without overlay must return empty array, got: {:?}",
        arr
    );
}
