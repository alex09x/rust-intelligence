/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/references part 5:
//! cross-file references in same crate, module hierarchies, and overlay handling.

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
fn test_references_cross_file_two_files_same_crate() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_r5_1", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_r5_1", "src/models.rs", "pub struct «SharedRecord»;\n", DefKind::Struct)
        .unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_refs_to_def("pkg_r5_1", "src/consumer_a.rs", "use crate::models::SharedRecord;\npub fn a() { let _ = «SharedRecord»; }\n", ids[0]).unwrap();
    ws.add_refs_to_def("pkg_r5_1", "src/consumer_b.rs", "use crate::models::SharedRecord;\npub fn b() { let _ = «SharedRecord»; }\n", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_r5_1").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let models_uri = ws.file_uri("pkg_r5_1", "src/models.rs").unwrap();
    let a_uri = ws.file_uri("pkg_r5_1", "src/consumer_a.rs").unwrap();
    let b_uri = ws.file_uri("pkg_r5_1", "src/consumer_b.rs").unwrap();

    let req = make_ref_req(&models_uri, 0, 11, false);
    let res = handle_references(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    let mut uris: Vec<_> = arr.iter().map(|l| l["uri"].as_str().unwrap()).collect();
    uris.sort();
    let mut expected = vec![a_uri.as_str(), b_uri.as_str()];
    expected.sort();
    assert_eq!(uris, expected);
}

#[test]
fn test_references_cross_file_three_files_same_crate() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_r5_2", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_r5_2", "src/defs.rs", "pub struct «TripleTarget»;\n", DefKind::Struct)
        .unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_refs_to_def("pkg_r5_2", "src/f1.rs", "«TripleTarget»;\n", ids[0]).unwrap();
    ws.add_refs_to_def("pkg_r5_2", "src/f2.rs", "«TripleTarget»;\n", ids[0]).unwrap();
    ws.add_refs_to_def("pkg_r5_2", "src/f3.rs", "«TripleTarget»;\n", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_r5_2").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let defs_uri = ws.file_uri("pkg_r5_2", "src/defs.rs").unwrap();

    let req = make_ref_req(&defs_uri, 0, 11, true);
    let res = handle_references(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 4);
}

#[test]
fn test_references_submodule_definition_referenced_in_root() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_r5_3", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_r5_3", "src/sub/inner.rs", "pub struct «InnerType»;\n", DefKind::Struct)
        .unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_refs_to_def("pkg_r5_3", "src/lib.rs", "use crate::sub::inner::InnerType;\npub fn root() { let _ = «InnerType»; }\n", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_r5_3").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let inner_uri = ws.file_uri("pkg_r5_3", "src/sub/inner.rs").unwrap();
    let lib_uri = ws.file_uri("pkg_r5_3", "src/lib.rs").unwrap();

    let req = make_ref_req(&inner_uri, 0, 11, false);
    let res = handle_references(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["uri"], lib_uri);
}

#[test]
fn test_references_root_definition_referenced_in_deep_submodule() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_r5_4", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_r5_4", "src/lib.rs", "pub struct «RootConfig»;\n", DefKind::Struct)
        .unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_refs_to_def("pkg_r5_4", "src/a/b/c.rs", "use crate::RootConfig;\npub fn leaf() { let _ = «RootConfig»; }\n", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_r5_4").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let lib_uri = ws.file_uri("pkg_r5_4", "src/lib.rs").unwrap();
    let c_uri = ws.file_uri("pkg_r5_4", "src/a/b/c.rs").unwrap();

    let req = make_ref_req(&lib_uri, 0, 11, false);
    let res = handle_references(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["uri"], c_uri);
}

#[test]
fn test_references_trait_definition_multiple_usages() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_r5_5", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_r5_5", "src/lib.rs", "pub trait «Displayable»;\n", DefKind::Trait)
        .unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub trait Displayable;\npub fn u1<T: «Displayable»>() {}\npub fn u2<T: «Displayable»>() {}\n";
    ws.add_refs_to_def("pkg_r5_5", "src/lib.rs", code, ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_r5_5").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_r5_5", "src/lib.rs").unwrap();

    let req = make_ref_req(&uri, 0, 10, true);
    let res = handle_references(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 3);
}

#[test]
fn test_references_with_clean_overlay_mapping() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_r5_6", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_r5_6", "src/lib.rs", "pub struct «CleanRefTarget»;\n", DefKind::Struct)
        .unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub struct CleanRefTarget;\npub fn run() { let _ = «CleanRefTarget»; }\n";
    ws.add_refs_to_def("pkg_r5_6", "src/lib.rs", code, ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_r5_6").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_r5_6", "src/lib.rs").unwrap();
    let file_path = ws.root_path().join("pkg_r5_6/src/lib.rs").display().to_string();

    overlay_mgr.open_with_provenance(&uri, &file_path, 1, code.to_string(), true);

    let req = make_ref_req(&uri, 0, 11, true);
    let res = handle_references(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 2);
}

#[test]
fn test_references_overlay_local_bindings_exact_positions() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let file = "/tmp/local_ref_test.rs";
    let uri = "file:///tmp/local_ref_test.rs";
    let code = "fn test_func() {\n    let counter = 10;\n    let _ = counter + 1;\n    let _ = counter + 2;\n}\n";

    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req_with_decl = make_ref_req(uri, 1, 8, true);
    let res_with_decl = handle_references(&req_with_decl, None, &overlay_mgr, &mut conv);
    let arr = res_with_decl.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0]["range"]["start"]["line"], 1);
    assert_eq!(arr[1]["range"]["start"]["line"], 2);
    assert_eq!(arr[2]["range"]["start"]["line"], 3);

    let req_no_decl = make_ref_req(uri, 1, 8, false);
    let res_no_decl = handle_references(&req_no_decl, None, &overlay_mgr, &mut conv);
    let arr_no = res_no_decl.as_array().unwrap();
    assert_eq!(arr_no.len(), 2);
    assert_eq!(arr_no[0]["range"]["start"]["line"], 2);
    assert_eq!(arr_no[1]["range"]["start"]["line"], 3);
}

#[test]
fn test_references_overlay_on_unrelated_file_preserves_target() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_r5_8", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_r5_8", "src/lib.rs", "pub struct «IsolatedRef»;\n", DefKind::Struct)
        .unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub struct IsolatedRef;\npub fn run() { let _ = «IsolatedRef»; }\n";
    ws.add_refs_to_def("pkg_r5_8", "src/lib.rs", code, ids[0]).unwrap();
    ws.add_source_file("pkg_r5_8", "src/other.rs", "// other file\n").unwrap();

    let shard = ws.build_crate_index("pkg_r5_8").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let lib_uri = ws.file_uri("pkg_r5_8", "src/lib.rs").unwrap();
    let other_uri = ws.file_uri("pkg_r5_8", "src/other.rs").unwrap();
    let other_path = ws.root_path().join("pkg_r5_8/src/other.rs").display().to_string();

    overlay_mgr.open_with_provenance(&other_uri, &other_path, 2, "// dirty other\n".to_string(), false);

    let req = make_ref_req(&lib_uri, 0, 11, true);
    let res = handle_references(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 2);
}

#[test]
fn test_references_generic_function_usages() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_r5_9", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_r5_9", "src/lib.rs", "pub fn «process_item»<T>(x: T) { let _ = x; }\n", DefKind::Fn)
        .unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub fn process_item<T>(x: T) { let _ = x; }\npub fn call() { «process_item»(42); }\n";
    ws.add_refs_to_def("pkg_r5_9", "src/lib.rs", code, ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_r5_9").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_r5_9", "src/lib.rs").unwrap();

    let req = make_ref_req(&uri, 0, 7, false);
    let res = handle_references(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["range"]["start"]["line"], 1);
}

#[test]
fn test_references_type_alias_usages() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_r5_10", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_r5_10", "src/lib.rs", "pub type «Handle» = usize;\n", DefKind::TyAlias)
        .unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub type Handle = usize;\npub fn use_h() -> «Handle» { 0 }\n";
    ws.add_refs_to_def("pkg_r5_10", "src/lib.rs", code, ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_r5_10").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_r5_10", "src/lib.rs").unwrap();

    let req = make_ref_req(&uri, 0, 9, false);
    let res = handle_references(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 1);
}
