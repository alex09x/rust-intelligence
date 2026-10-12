/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/implementation part 6:
//! cross-crate implementations, multi-shard indices, and overlay provenance states.

use ridx::lsp::navigation::handle_implementation;
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
fn test_implementation_cross_crate_trait_implemented_in_downstream() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("core_pkg", "0.1.0").unwrap();
    ws.add_crate("app_pkg", "0.1.0").unwrap();

    let (_, ids) = ws
        .add_source_file_with_defs("core_pkg", "src/lib.rs", "pub trait «CorePlugin»;\n", DefKind::Trait)
        .unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file_with_impl_refs(
        "app_pkg",
        "src/lib.rs",
        "use core_pkg::CorePlugin;\n«impl CorePlugin for App» {}\n",
        "core_pkg",
        ids[0],
    )
    .unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let core_uri = ws.file_uri("core_pkg", "src/lib.rs").unwrap();
    let app_uri = ws.file_uri("app_pkg", "src/lib.rs").unwrap();

    let req = make_pos_req(&core_uri, 0, 11);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["uri"], app_uri);
}

#[test]
fn test_implementation_cross_crate_three_shards_all_implementing() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("api_pkg", "0.1.0").unwrap();
    ws.add_crate("backend_a", "0.1.0").unwrap();
    ws.add_crate("backend_b", "0.1.0").unwrap();

    let (_, ids) = ws
        .add_source_file_with_defs("api_pkg", "src/lib.rs", "pub trait «Backend»;\n", DefKind::Trait)
        .unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file_with_impl_refs("backend_a", "src/lib.rs", "«impl Backend for ServiceA» {}\n", "api_pkg", ids[0]).unwrap();
    ws.add_source_file_with_impl_refs("backend_b", "src/lib.rs", "«impl Backend for ServiceB» {}\n", "api_pkg", ids[0]).unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let api_uri = ws.file_uri("api_pkg", "src/lib.rs").unwrap();
    let a_uri = ws.file_uri("backend_a", "src/lib.rs").unwrap();
    let b_uri = ws.file_uri("backend_b", "src/lib.rs").unwrap();

    let req = make_pos_req(&api_uri, 0, 11);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    let mut uris: Vec<_> = arr.iter().map(|l| l["uri"].as_str().unwrap()).collect();
    uris.sort();
    let mut expected = vec![a_uri.as_str(), b_uri.as_str()];
    expected.sort();
    assert_eq!(uris, expected);
}

#[test]
fn test_implementation_clean_overlay_returns_accurate_locations() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_i6_clean", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_i6_clean", "src/lib.rs", "pub trait «CleanTrait»;\n", DefKind::Trait)
        .unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub trait CleanTrait;\n«impl CleanTrait for CleanStruct» {}\n";
    ws.add_source_file_with_impl_refs("pkg_i6_clean", "src/lib.rs", code, "pkg_i6_clean", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_i6_clean").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_i6_clean", "src/lib.rs").unwrap();
    let file_path = ws.root_path().join("pkg_i6_clean/src/lib.rs").display().to_string();

    overlay_mgr.open_with_provenance(&uri, &file_path, 1, code.to_string(), true);

    let req = make_pos_req(&uri, 0, 11);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["uri"], uri);
}

#[test]
fn test_implementation_dirty_overlay_abstained_returns_empty() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_i6_dirty", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_i6_dirty", "src/lib.rs", "pub trait «DirtyTrait»;\n", DefKind::Trait)
        .unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub trait DirtyTrait;\n«impl DirtyTrait for DirtyStruct» {}\n";
    ws.add_source_file_with_impl_refs("pkg_i6_dirty", "src/lib.rs", code, "pkg_i6_dirty", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_i6_dirty").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_i6_dirty", "src/lib.rs").unwrap();
    let file_path = ws.root_path().join("pkg_i6_dirty/src/lib.rs").display().to_string();

    overlay_mgr.open_with_provenance(&uri, &file_path, 1, "pub trait DirtyTrait;\n// modified\n".to_string(), false);

    let req = make_pos_req(&uri, 0, 11);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]));
}

#[test]
fn test_implementation_overlay_on_different_file_preserves_target() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_i6_diff", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_i6_diff", "src/traits.rs", "pub trait «TargetTrait»;\n", DefKind::Trait)
        .unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file_with_impl_refs("pkg_i6_diff", "src/impls.rs", "«impl TargetTrait for StructX» {}\n", "pkg_i6_diff", ids[0]).unwrap();
    ws.add_source_file("pkg_i6_diff", "src/unrelated.rs", "// unrelated file\n").unwrap();

    let shard = ws.build_crate_index("pkg_i6_diff").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let traits_uri = ws.file_uri("pkg_i6_diff", "src/traits.rs").unwrap();
    let unrelated_uri = ws.file_uri("pkg_i6_diff", "src/unrelated.rs").unwrap();
    let unrel_path = ws.root_path().join("pkg_i6_diff/src/unrelated.rs").display().to_string();

    // Dirty overlay on unrelated file should not affect traits.rs query
    overlay_mgr.open_with_provenance(&unrelated_uri, &unrel_path, 2, "// dirty unrelated\n".to_string(), false);

    let req = make_pos_req(&traits_uri, 0, 11);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 1);
}

#[test]
fn test_implementation_multiple_crates_same_trait_name_isolation() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("crate_alpha", "0.1.0").unwrap();
    ws.add_crate("crate_beta", "0.1.0").unwrap();

    let (_, ids_a) = ws
        .add_source_file_with_defs("crate_alpha", "src/lib.rs", "pub trait «IsolatedTrait»;\n", DefKind::Trait)
        .unwrap();
    let (_, ids_b) = ws
        .add_source_file_with_defs("crate_beta", "src/lib.rs", "pub trait «IsolatedTrait»;\n", DefKind::Trait)
        .unwrap();

    ws.add_source_file_with_impl_refs("crate_alpha", "src/lib.rs", "pub trait IsolatedTrait;\n«impl IsolatedTrait for A» {}\n", "crate_alpha", ids_a[0]).unwrap();
    ws.add_source_file_with_impl_refs("crate_beta", "src/lib.rs", "pub trait IsolatedTrait;\n«impl IsolatedTrait for B» {}\n", "crate_beta", ids_b[0]).unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let a_uri = ws.file_uri("crate_alpha", "src/lib.rs").unwrap();

    let req = make_pos_req(&a_uri, 0, 11);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["uri"], a_uri);
}

#[test]
fn test_implementation_multiple_crates_downstream_blanket_impl() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("lib_base", "0.1.0").unwrap();
    ws.add_crate("lib_ext", "0.1.0").unwrap();

    let (_, ids) = ws
        .add_source_file_with_defs("lib_base", "src/lib.rs", "pub trait «Extension»;\n", DefKind::Trait)
        .unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file_with_impl_refs("lib_ext", "src/lib.rs", "«impl<T> Extension for T» {}\n", "lib_base", ids[0]).unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let base_uri = ws.file_uri("lib_base", "src/lib.rs").unwrap();
    let ext_uri = ws.file_uri("lib_ext", "src/lib.rs").unwrap();

    let req = make_pos_req(&base_uri, 0, 11);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["uri"], ext_uri);
}

#[test]
fn test_implementation_reexported_trait_implementation() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_reexp", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_reexp", "src/internal.rs", "pub trait «InternalTrait»;\n", DefKind::Trait)
        .unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file_with_impl_refs("pkg_reexp", "src/lib.rs", "pub use crate::internal::InternalTrait;\n«impl InternalTrait for ()» {}\n", "pkg_reexp", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_reexp").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let internal_uri = ws.file_uri("pkg_reexp", "src/internal.rs").unwrap();
    let lib_uri = ws.file_uri("pkg_reexp", "src/lib.rs").unwrap();

    let req = make_pos_req(&internal_uri, 0, 11);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["uri"], lib_uri);
}

#[test]
fn test_implementation_cross_crate_struct_inherent_in_downstream() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("upstream_c", "0.1.0").unwrap();
    ws.add_crate("downstream_c", "0.1.0").unwrap();

    let (_, ids) = ws
        .add_source_file_with_defs("upstream_c", "src/lib.rs", "pub trait «ExportedContract»;\n", DefKind::Trait)
        .unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file_with_impl_refs("downstream_c", "src/lib.rs", "«impl ExportedContract for DownstreamHandler» {}\n", "upstream_c", ids[0]).unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let up_uri = ws.file_uri("upstream_c", "src/lib.rs").unwrap();
    let down_uri = ws.file_uri("downstream_c", "src/lib.rs").unwrap();

    let req = make_pos_req(&up_uri, 0, 11);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["uri"], down_uri);
}
