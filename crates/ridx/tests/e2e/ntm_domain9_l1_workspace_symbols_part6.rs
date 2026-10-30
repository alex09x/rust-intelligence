/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspace/symbol part 6:
//! cross-crate search, container names, location URIs, and range coordinate validation.

use ridx::lsp::protocol::PositionConverter;
use ridx::lsp::symbols::handle_workspace_symbol;
use ridx_format::{DefKind, WorkspaceIndex};
use ridx_test_support::fixture::SyntheticWorkspaceFixture;
use serde_json::json;

#[test]
fn test_workspace_symbol_cross_crate_disjoint_names() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("crate_alpha", "0.1.0").unwrap();
    ws.add_source_file_with_defs("crate_alpha", "src/lib.rs", "pub struct «RecordAlpha»;\n", DefKind::Struct).unwrap();

    ws.add_crate("crate_beta", "0.1.0").unwrap();
    ws.add_source_file_with_defs("crate_beta", "src/lib.rs", "pub struct «RecordBeta»;\n", DefKind::Struct).unwrap();

    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "query": "Record" }
    });
    let res = handle_workspace_symbol(&req, [&idx].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["name"], "RecordAlpha");
    assert_eq!(items[1]["name"], "RecordBeta");
}

#[test]
fn test_workspace_symbol_container_name_root_module() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_root_mod", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_root_mod", "src/lib.rs", "pub struct «RootStruct»;\n", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "query": "RootStruct" }
    });
    let res = handle_workspace_symbol(&req, [&idx].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["containerName"], "pkg_root_mod");
}

#[test]
fn test_workspace_symbol_container_name_single_submodule() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_submod", "0.1.0").unwrap();
    ws.add_source_file_with_prefix_defs("pkg_submod", "src/storage.rs", "pub struct «StorageEngine»;\n", "pkg_submod::storage", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "query": "StorageEngine" }
    });
    let res = handle_workspace_symbol(&req, [&idx].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["containerName"], "storage");
}

#[test]
fn test_workspace_symbol_uri_scheme_and_prefix() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_uri_check", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_uri_check", "src/lib.rs", "pub struct «TargetEntity»;\n", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "query": "TargetEntity" }
    });
    let res = handle_workspace_symbol(&req, [&idx].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 1);
    let uri = items[0]["location"]["uri"].as_str().unwrap();
    assert!(uri.starts_with("file://"));
}

#[test]
fn test_workspace_symbol_range_start_end_valid() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_range_val", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_range_val", "src/lib.rs", "pub struct «ValidRangeRecord»;\n", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "query": "ValidRangeRecord" }
    });
    let res = handle_workspace_symbol(&req, [&idx].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 1);
    let range = &items[0]["location"]["range"];
    assert_eq!(range["start"]["line"], 0);
    assert_eq!(range["start"]["character"], 11);
    assert_eq!(range["end"]["line"], 0);
    assert_eq!(range["end"]["character"], 27);
}

#[test]
fn test_workspace_symbol_multi_file_same_crate() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_multifile", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_multifile", "src/lib.rs", "pub struct «ItemInLib»;\n", DefKind::Struct).unwrap();
    ws.add_source_file_with_defs("pkg_multifile", "src/models.rs", "pub struct «ItemInModels»;\n", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "query": "ItemIn" }
    });
    let res = handle_workspace_symbol(&req, [&idx].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["name"], "ItemInLib");
    assert_eq!(items[1]["name"], "ItemInModels");
}

#[test]
fn test_workspace_symbol_empty_query_lists_all_symbols() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_all_sym", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_all_sym", "src/lib.rs", "pub struct «SymOne»;\npub struct «SymTwo»;\n", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "query": "" }
    });
    let res = handle_workspace_symbol(&req, [&idx].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 2);
}

#[test]
fn test_workspace_symbol_missing_query_field_treated_as_empty() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_missing_q", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_missing_q", "src/lib.rs", "pub struct «DefaultItem»;\n", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": {}
    });
    let res = handle_workspace_symbol(&req, [&idx].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "DefaultItem");
}
