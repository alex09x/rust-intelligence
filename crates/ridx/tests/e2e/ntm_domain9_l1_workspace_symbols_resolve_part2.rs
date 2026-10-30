/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspaceSymbol/resolve part 2:
//! resolution of structs, enums, functions, traits, constants, statics, type aliases, and metadata.

use ridx::lsp::protocol::PositionConverter;
use ridx::lsp::symbols::handle_workspace_symbol_resolve;
use ridx_format::{DefKind, WorkspaceIndex};
use ridx_test_support::fixture::SyntheticWorkspaceFixture;
use serde_json::json;

#[test]
fn test_workspace_symbol_resolve_struct_def_kind() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_struct_res", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_struct_res", "src/lib.rs", "pub struct «ResolvedStruct»;\n", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "name": "ResolvedStruct" }
    });
    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&idx), &mut conv);
    assert_eq!(res["name"], "ResolvedStruct");
    assert_eq!(res["containerName"], "pkg_struct_res");
    assert_eq!(res["location"]["range"]["start"]["line"], 0);
    assert_eq!(res["location"]["range"]["start"]["character"], 11);
    assert_eq!(res["location"]["range"]["end"]["character"], 25);
}

#[test]
fn test_workspace_symbol_resolve_enum_def_kind() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_enum_res", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_enum_res", "src/lib.rs", "pub enum «ResolvedEnum» { OptionA }\n", DefKind::Enum).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "name": "ResolvedEnum" }
    });
    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&idx), &mut conv);
    assert_eq!(res["name"], "ResolvedEnum");
    assert_eq!(res["containerName"], "pkg_enum_res");
    assert_eq!(res["location"]["range"]["start"]["line"], 0);
}

#[test]
fn test_workspace_symbol_resolve_fn_def_kind() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_fn_res", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_fn_res", "src/lib.rs", "pub fn «resolved_fn»() {}\n", DefKind::Fn).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "name": "resolved_fn" }
    });
    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&idx), &mut conv);
    assert_eq!(res["name"], "resolved_fn");
    assert_eq!(res["containerName"], "pkg_fn_res");
    assert_eq!(res["location"]["range"]["start"]["line"], 0);
}

#[test]
fn test_workspace_symbol_resolve_trait_def_kind() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_trait_res", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_trait_res", "src/lib.rs", "pub trait «ResolvedTrait» {}\n", DefKind::Trait).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "name": "ResolvedTrait" }
    });
    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&idx), &mut conv);
    assert_eq!(res["name"], "ResolvedTrait");
    assert_eq!(res["containerName"], "pkg_trait_res");
    assert_eq!(res["location"]["range"]["start"]["line"], 0);
}

#[test]
fn test_workspace_symbol_resolve_const_def_kind() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_const_res", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_const_res", "src/lib.rs", "pub const «RESOLVED_CONST»: u32 = 42;\n", DefKind::Const).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "name": "RESOLVED_CONST" }
    });
    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&idx), &mut conv);
    assert_eq!(res["name"], "RESOLVED_CONST");
    assert_eq!(res["containerName"], "pkg_const_res");
    assert_eq!(res["location"]["range"]["start"]["line"], 0);
}

#[test]
fn test_workspace_symbol_resolve_static_def_kind() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_static_res", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_static_res", "src/lib.rs", "pub static «RESOLVED_STATIC»: u32 = 100;\n", DefKind::Static).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "name": "RESOLVED_STATIC" }
    });
    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&idx), &mut conv);
    assert_eq!(res["name"], "RESOLVED_STATIC");
    assert_eq!(res["containerName"], "pkg_static_res");
    assert_eq!(res["location"]["range"]["start"]["line"], 0);
}

#[test]
fn test_workspace_symbol_resolve_type_alias_def_kind() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_alias_res", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_alias_res", "src/lib.rs", "pub type «ResolvedAlias» = i64;\n", DefKind::TyAlias).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "name": "ResolvedAlias" }
    });
    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&idx), &mut conv);
    assert_eq!(res["name"], "ResolvedAlias");
    assert_eq!(res["containerName"], "pkg_alias_res");
    assert_eq!(res["location"]["range"]["start"]["line"], 0);
}

#[test]
fn test_workspace_symbol_resolve_mod_def_kind() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_mod_res", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_mod_res", "src/lib.rs", "pub mod «resolved_mod»;\n", DefKind::Mod).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "name": "resolved_mod" }
    });
    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&idx), &mut conv);
    assert_eq!(res["name"], "resolved_mod");
    assert_eq!(res["containerName"], "pkg_mod_res");
    assert_eq!(res["location"]["range"]["start"]["line"], 0);
}

#[test]
fn test_workspace_symbol_resolve_preserves_unrelated_custom_field() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_custom", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_custom", "src/lib.rs", "pub struct «CustomPayload»;\n", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": {
            "name": "CustomPayload",
            "data": { "clientId": 99 },
            "customHeader": "preserve_me"
        }
    });
    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&idx), &mut conv);
    assert_eq!(res["name"], "CustomPayload");
    assert_eq!(res["data"]["clientId"], 99);
    assert_eq!(res["customHeader"], "preserve_me");
    assert_eq!(res["location"]["range"]["start"]["line"], 0);
}
