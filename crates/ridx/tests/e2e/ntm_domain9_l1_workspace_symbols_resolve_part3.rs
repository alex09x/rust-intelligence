/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspaceSymbol/resolve part 3:
//! existing location preservation, null/missing location replacement, and error safety.

use ridx::lsp::protocol::PositionConverter;
use ridx::lsp::symbols::handle_workspace_symbol_resolve;
use ridx_format::{DefKind, WorkspaceIndex};
use ridx_test_support::fixture::SyntheticWorkspaceFixture;
use serde_json::{json, Value};

#[test]
fn test_workspace_symbol_resolve_preserves_existing_location() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_pres_loc", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_pres_loc", "src/lib.rs", "pub struct «PreserveTarget»;\n", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let custom_loc = json!({
        "uri": "file:///custom/existing.rs",
        "range": {
            "start": { "line": 99, "character": 0 },
            "end": { "line": 99, "character": 10 }
        }
    });
    let req = json!({
        "params": {
            "name": "PreserveTarget",
            "location": custom_loc
        }
    });
    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&idx), &mut conv);
    assert_eq!(res["name"], "PreserveTarget");
    assert_eq!(res["location"]["uri"], "file:///custom/existing.rs");
    assert_eq!(res["location"]["range"]["start"]["line"], 99);
}

#[test]
fn test_workspace_symbol_resolve_null_location_is_replaced() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_null_loc", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_null_loc", "src/lib.rs", "pub struct «ReplaceMe»;\n", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": {
            "name": "ReplaceMe",
            "location": Value::Null
        }
    });
    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&idx), &mut conv);
    assert_eq!(res["name"], "ReplaceMe");
    assert_eq!(res["location"]["range"]["start"]["line"], 0);
}

#[test]
fn test_workspace_symbol_resolve_missing_location_is_populated() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_miss_loc", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_miss_loc", "src/lib.rs", "pub struct «PopulateMe»;\n", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": {
            "name": "PopulateMe"
        }
    });
    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&idx), &mut conv);
    assert_eq!(res["name"], "PopulateMe");
    assert_eq!(res["location"]["range"]["start"]["line"], 0);
}

#[test]
fn test_workspace_symbol_resolve_non_existent_symbol_unchanged() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_nonexist", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_nonexist", "src/lib.rs", "pub struct «RealSymbol»;\n", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": {
            "name": "DoesNotExistAtAll"
        }
    });
    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&idx), &mut conv);
    assert_eq!(res["name"], "DoesNotExistAtAll");
    assert!(res.get("location").is_none_or(Value::is_null));
}

#[test]
fn test_workspace_symbol_resolve_null_params_returns_null() {
    let mut conv = PositionConverter::new();
    let empty_indices: Vec<&WorkspaceIndex> = Vec::new();
    let req = json!({
        "params": Value::Null
    });
    let res = handle_workspace_symbol_resolve(&req, empty_indices.into_iter(), &mut conv);
    assert_eq!(res, Value::Null);
}

#[test]
fn test_workspace_symbol_resolve_missing_params_returns_null() {
    let mut conv = PositionConverter::new();
    let empty_indices: Vec<&WorkspaceIndex> = Vec::new();
    let req = json!({});
    let res = handle_workspace_symbol_resolve(&req, empty_indices.into_iter(), &mut conv);
    assert_eq!(res, Value::Null);
}

#[test]
fn test_workspace_symbol_resolve_preserves_symbol_kind_field() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_kind_pres", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_kind_pres", "src/lib.rs", "pub struct «KindPreserve»;\n", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": {
            "name": "KindPreserve",
            "kind": 23
        }
    });
    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&idx), &mut conv);
    assert_eq!(res["name"], "KindPreserve");
    assert_eq!(res["kind"], 23);
    assert_eq!(res["location"]["range"]["start"]["line"], 0);
}

#[test]
fn test_workspace_symbol_resolve_empty_name_returns_unchanged() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_empty_name", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_empty_name", "src/lib.rs", "pub struct «SomeItem»;\n", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": {
            "name": ""
        }
    });
    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&idx), &mut conv);
    assert_eq!(res["name"], "");
    assert!(res.get("location").is_none_or(Value::is_null));
}

#[test]
fn test_workspace_symbol_resolve_empty_shards_list_returns_unchanged() {
    let mut conv = PositionConverter::new();
    let empty_indices: Vec<&WorkspaceIndex> = Vec::new();
    let req = json!({
        "params": {
            "name": "TargetInNoShards"
        }
    });
    let res = handle_workspace_symbol_resolve(&req, empty_indices.into_iter(), &mut conv);
    assert_eq!(res["name"], "TargetInNoShards");
    assert!(res.get("location").is_none_or(Value::is_null));
}
