/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspace/symbol part 5:
//! prefix, suffix, substring, case-insensitivity, and kind mapping queries.

use ridx::lsp::protocol::PositionConverter;
use ridx::lsp::symbols::handle_workspace_symbol;
use ridx_format::{DefKind, WorkspaceIndex};
use ridx_test_support::fixture::SyntheticWorkspaceFixture;
use serde_json::json;

#[test]
fn test_workspace_symbol_exact_prefix_query() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("crate_items", "0.1.0").unwrap();
    ws.add_source_file_with_defs("crate_items", "src/lib.rs", "pub struct «PrefixAlpha»;\npub struct «PrefixBeta»;\npub struct «OtherGamma»;\n", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "query": "Prefix" }
    });
    let res = handle_workspace_symbol(&req, [&idx].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["name"], "PrefixAlpha");
    assert_eq!(items[1]["name"], "PrefixBeta");
}

#[test]
fn test_workspace_symbol_suffix_matching() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_suffix", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_suffix", "src/lib.rs", "pub struct «TaskHandler»;\npub struct «RequestHandler»;\npub struct «WorkerService»;\n", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "query": "Handler" }
    });
    let res = handle_workspace_symbol(&req, [&idx].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["name"], "TaskHandler");
    assert_eq!(items[1]["name"], "RequestHandler");
}

#[test]
fn test_workspace_symbol_substring_matching_middle() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_substr", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_substr", "src/lib.rs", "pub struct «AwesomeBuilderRecord»;\npub struct «SimpleBuilder»;\npub struct «OtherItem»;\n", DefKind::Struct)
        .unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "query": "Builder" }
    });
    let res = handle_workspace_symbol(&req, [&idx].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["name"], "AwesomeBuilderRecord");
    assert_eq!(items[1]["name"], "SimpleBuilder");
}

#[test]
fn test_workspace_symbol_case_insensitive_uppercase_query() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_case_up", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_case_up", "src/lib.rs", "pub struct «UserAccount»;\n", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "query": "USERACCOUNT" }
    });
    let res = handle_workspace_symbol(&req, [&idx].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "UserAccount");
}

#[test]
fn test_workspace_symbol_case_insensitive_lowercase_query() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_case_low", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_case_low", "src/lib.rs", "pub const «GLOBAL_TIMEOUT»: u64 = 30;\n", DefKind::Const).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "query": "global_timeout" }
    });
    let res = handle_workspace_symbol(&req, [&idx].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "GLOBAL_TIMEOUT");
}

#[test]
fn test_workspace_symbol_all_def_kinds_present() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_kinds", "0.1.0").unwrap();
    ws.add_source_file_with_defs(
        "pkg_kinds",
        "src/lib.rs",
        "pub struct «MyStruct»;\npub enum «MyEnum» { A }\npub fn «my_fn»() {}\npub trait «MyTrait» {}\npub const «MY_CONST»: i32 = 1;\npub type «MyAlias» = i32;\n",
        DefKind::Struct,
    )
    .unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "query": "My" }
    });
    let res = handle_workspace_symbol(&req, [&idx].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 6);
}

#[test]
fn test_workspace_symbol_empty_workspace_returns_empty() {
    let _ws = SyntheticWorkspaceFixture::new().unwrap();
    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "query": "anything" }
    });
    let empty_indices: Vec<&WorkspaceIndex> = Vec::new();
    let res = handle_workspace_symbol(&req, empty_indices.into_iter(), &mut conv);
    assert_eq!(res, json!([]));
}

#[test]
fn test_workspace_symbol_no_matches_returns_empty_array() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_nomatch", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_nomatch", "src/lib.rs", "pub struct «TargetFoo»;\n", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "query": "TotallyUnrelatedQuery" }
    });
    let res = handle_workspace_symbol(&req, [&idx].into_iter(), &mut conv);
    assert_eq!(res, json!([]));
}
