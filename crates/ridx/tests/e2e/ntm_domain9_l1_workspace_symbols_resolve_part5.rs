/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspaceSymbol/resolve part 5:
//! associated functions, constants, fields, complex payloads, idempotence, and exact matching.

use ridx::lsp::protocol::PositionConverter;
use ridx::lsp::symbols::handle_workspace_symbol_resolve;
use ridx_format::{DefKind, WorkspaceIndex};
use ridx_test_support::fixture::SyntheticWorkspaceFixture;
use serde_json::json;

#[test]
fn test_workspace_symbol_resolve_field_item_in_struct() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_field_res", "0.1.0").unwrap();
    ws.add_source_file_with_defs(
        "pkg_field_res",
        "src/lib.rs",
        "pub struct Record { pub «count_field»: usize }\n",
        DefKind::Field,
    )
    .unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "name": "count_field" }
    });
    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&idx), &mut conv);
    assert_eq!(res["name"], "count_field");
    assert_eq!(res["containerName"], "pkg_field_res");
    assert_eq!(res["location"]["range"]["start"]["line"], 0);
}

#[test]
fn test_workspace_symbol_resolve_assoc_fn_in_impl() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_assoc_fn", "0.1.0").unwrap();
    ws.add_source_file_with_defs(
        "pkg_assoc_fn",
        "src/lib.rs",
        "impl Service { pub fn «serve_request»() {} }\n",
        DefKind::AssocFn,
    )
    .unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "name": "serve_request" }
    });
    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&idx), &mut conv);
    assert_eq!(res["name"], "serve_request");
    assert_eq!(res["containerName"], "pkg_assoc_fn");
    assert_eq!(res["location"]["range"]["start"]["line"], 0);
}

#[test]
fn test_workspace_symbol_resolve_assoc_const_in_trait() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_assoc_c", "0.1.0").unwrap();
    ws.add_source_file_with_defs(
        "pkg_assoc_c",
        "src/lib.rs",
        "pub trait Protocol { const «VERSION_CODE»: u32; }\n",
        DefKind::AssocConst,
    )
    .unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "name": "VERSION_CODE" }
    });
    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&idx), &mut conv);
    assert_eq!(res["name"], "VERSION_CODE");
    assert_eq!(res["containerName"], "pkg_assoc_c");
    assert_eq!(res["location"]["range"]["start"]["line"], 0);
}

#[test]
fn test_workspace_symbol_resolve_complex_client_nested_json() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_complex", "0.1.0").unwrap();
    ws.add_source_file_with_defs(
        "pkg_complex",
        "src/lib.rs",
        "pub struct «ComplexTarget»;\n",
        DefKind::Struct,
    )
    .unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": {
            "name": "ComplexTarget",
            "clientContext": {
                "nested": {
                    "tags": ["a", "b", "c"]
                }
            }
        }
    });
    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&idx), &mut conv);
    assert_eq!(res["name"], "ComplexTarget");
    assert_eq!(res["clientContext"]["nested"]["tags"][0], "a");
    assert_eq!(res["clientContext"]["nested"]["tags"][1], "b");
    assert_eq!(res["clientContext"]["nested"]["tags"][2], "c");
    assert_eq!(res["location"]["range"]["start"]["line"], 0);
}

#[test]
fn test_workspace_symbol_resolve_four_shards_fourth_shard_lookup() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("c1", "0.1.0").unwrap();
    ws.add_source_file_with_defs("c1", "src/lib.rs", "pub struct «S1»;\n", DefKind::Struct).unwrap();
    ws.add_crate("c2", "0.1.0").unwrap();
    ws.add_source_file_with_defs("c2", "src/lib.rs", "pub struct «S2»;\n", DefKind::Struct).unwrap();
    ws.add_crate("c3", "0.1.0").unwrap();
    ws.add_source_file_with_defs("c3", "src/lib.rs", "pub struct «S3»;\n", DefKind::Struct).unwrap();
    ws.add_crate("c4", "0.1.0").unwrap();
    ws.add_source_file_with_defs("c4", "src/lib.rs", "pub struct «TargetInC4»;\n", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "name": "TargetInC4" }
    });
    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&idx), &mut conv);
    assert_eq!(res["name"], "TargetInC4");
    assert_eq!(res["containerName"], "c4");
}

#[test]
fn test_workspace_symbol_resolve_non_ascii_unicode_symbol_name() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_german", "0.1.0").unwrap();
    ws.add_source_file_with_defs(
        "pkg_german",
        "src/lib.rs",
        "pub struct «Übersicht»;\n",
        DefKind::Struct,
    )
    .unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "name": "Übersicht" }
    });
    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&idx), &mut conv);
    assert_eq!(res["name"], "Übersicht");
    assert_eq!(res["location"]["range"]["start"]["line"], 0);
}

#[test]
fn test_workspace_symbol_resolve_repeated_consecutive_resolves_idempotent() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_idempotent", "0.1.0").unwrap();
    ws.add_source_file_with_defs(
        "pkg_idempotent",
        "src/lib.rs",
        "pub struct «IdempotentItem»;\n",
        DefKind::Struct,
    )
    .unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req1 = json!({
        "params": { "name": "IdempotentItem" }
    });
    let res1 = handle_workspace_symbol_resolve(&req1, std::iter::once(&idx), &mut conv);
    let req2 = json!({
        "params": res1
    });
    let res2 = handle_workspace_symbol_resolve(&req2, std::iter::once(&idx), &mut conv);
    assert_eq!(res1, res2);
}

#[test]
fn test_workspace_symbol_resolve_root_manifest_workspace_indexing() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("root_member", "0.1.0").unwrap();
    ws.add_source_file_with_defs(
        "root_member",
        "src/lib.rs",
        "pub struct «MemberStruct»;\n",
        DefKind::Struct,
    )
    .unwrap();
    ws.write_workspace_manifest().unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "name": "MemberStruct" }
    });
    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&idx), &mut conv);
    assert_eq!(res["name"], "MemberStruct");
    assert_eq!(res["containerName"], "root_member");
}

#[test]
fn test_workspace_symbol_resolve_exact_match_priority_over_prefix() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_exact_prio", "0.1.0").unwrap();
    ws.add_source_file_with_defs(
        "pkg_exact_prio",
        "src/lib.rs",
        "pub struct «Target»;\npub struct «TargetLonger»;\n",
        DefKind::Struct,
    )
    .unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "name": "Target" }
    });
    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&idx), &mut conv);
    assert_eq!(res["name"], "Target");
}
