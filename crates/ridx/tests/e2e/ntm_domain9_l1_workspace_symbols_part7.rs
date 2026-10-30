/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspace/symbol part 7:
//! shard deduplication, multi-shard ordering, underscores, and deep nesting.

use ridx::lsp::protocol::PositionConverter;
use ridx::lsp::symbols::handle_workspace_symbol;
use ridx_format::{DefKind, WorkspaceIndex};
use ridx_test_support::fixture::SyntheticWorkspaceFixture;
use serde_json::json;

#[test]
fn test_workspace_symbol_identical_index_deduplicated() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_dedupe_test", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_dedupe_test", "src/lib.rs", "pub struct «DedupeItem»;\n", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "query": "DedupeItem" }
    });
    // Passing the same index reference three times
    let res = handle_workspace_symbol(&req, [&idx, &idx, &idx].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "DedupeItem");
}

#[test]
fn test_workspace_symbol_three_shards_ordered_merge() {
    let mut ws_1 = SyntheticWorkspaceFixture::new().unwrap();
    ws_1.add_crate("crate_1", "0.1.0").unwrap();
    ws_1.add_source_file_with_defs("crate_1", "src/lib.rs", "pub struct «ShardOneItem»;\n", DefKind::Struct).unwrap();
    ws_1.build_all_indices().unwrap();
    let idx_1 = WorkspaceIndex::open(ws_1.root_path()).unwrap();

    let mut ws_2 = SyntheticWorkspaceFixture::new().unwrap();
    ws_2.add_crate("crate_2", "0.1.0").unwrap();
    ws_2.add_source_file_with_defs("crate_2", "src/lib.rs", "pub struct «ShardTwoItem»;\n", DefKind::Struct).unwrap();
    ws_2.build_all_indices().unwrap();
    let idx_2 = WorkspaceIndex::open(ws_2.root_path()).unwrap();

    let mut ws_3 = SyntheticWorkspaceFixture::new().unwrap();
    ws_3.add_crate("crate_3", "0.1.0").unwrap();
    ws_3.add_source_file_with_defs("crate_3", "src/lib.rs", "pub struct «ShardThreeItem»;\n", DefKind::Struct).unwrap();
    ws_3.build_all_indices().unwrap();
    let idx_3 = WorkspaceIndex::open(ws_3.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({ "params": { "query": "Shard" } });
    let res = handle_workspace_symbol(&req, [&idx_1, &idx_2, &idx_3].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["name"], "ShardOneItem");
    assert_eq!(items[1]["name"], "ShardTwoItem");
    assert_eq!(items[2]["name"], "ShardThreeItem");
}

#[test]
fn test_workspace_symbol_query_with_underscore_matching() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_under", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_under", "src/lib.rs", "pub struct «my_internal_helper»;\n", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "query": "internal" }
    });
    let res = handle_workspace_symbol(&req, [&idx].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "my_internal_helper");
}

#[test]
fn test_workspace_symbol_query_with_numbers() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_num", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_num", "src/lib.rs", "pub struct «PacketV1»;\npub struct «PacketV2»;\n", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "query": "V1" }
    });
    let res = handle_workspace_symbol(&req, [&idx].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "PacketV1");
}

#[test]
fn test_workspace_symbol_same_name_different_crates_preserved() {
    let mut ws_a = SyntheticWorkspaceFixture::new().unwrap();
    ws_a.add_crate("crate_first", "0.1.0").unwrap();
    ws_a.add_source_file_with_defs("crate_first", "src/lib.rs", "pub struct «SharedConfig»;\n", DefKind::Struct).unwrap();
    ws_a.build_all_indices().unwrap();
    let idx_a = WorkspaceIndex::open(ws_a.root_path()).unwrap();

    let mut ws_b = SyntheticWorkspaceFixture::new().unwrap();
    ws_b.add_crate("crate_second", "0.1.0").unwrap();
    ws_b.add_source_file_with_defs("crate_second", "src/lib.rs", "pub struct «SharedConfig»;\n", DefKind::Struct).unwrap();
    ws_b.build_all_indices().unwrap();
    let idx_b = WorkspaceIndex::open(ws_b.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({ "params": { "query": "SharedConfig" } });
    let res = handle_workspace_symbol(&req, [&idx_a, &idx_b].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["name"], "SharedConfig");
    assert_eq!(items[1]["name"], "SharedConfig");
    // Verify exact expected file URIs
    let expected_a = ws_a.file_uri("crate_first", "src/lib.rs").unwrap();
    let expected_b = ws_b.file_uri("crate_second", "src/lib.rs").unwrap();
    let uri_0 = items[0]["location"]["uri"].as_str().unwrap();
    let uri_1 = items[1]["location"]["uri"].as_str().unwrap();
    assert_eq!(std::collections::HashSet::from([uri_0, uri_1]), std::collections::HashSet::from([expected_a.as_str(), expected_b.as_str()]));
}

#[test]
fn test_workspace_symbol_submodule_deep_nesting() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_deep", "0.1.0").unwrap();
    ws.add_source_file_with_prefix_defs("pkg_deep", "src/a/b/c.rs", "pub struct «NestedLeaf»;\n", "pkg_deep::a::b::c", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "query": "NestedLeaf" }
    });
    let res = handle_workspace_symbol(&req, [&idx].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["containerName"], "c");
}

#[test]
fn test_workspace_symbol_root_manifest_workspace_indexing() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("app", "0.1.0").unwrap();
    ws.add_source_file_with_defs("app", "src/lib.rs", "pub struct «AppEntry»;\n", DefKind::Struct).unwrap();
    ws.write_workspace_manifest().unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "query": "AppEntry" }
    });
    let res = handle_workspace_symbol(&req, [&idx].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "AppEntry");
}

#[test]
fn test_workspace_symbol_query_matching_start_of_symbol() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_start", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_start", "src/lib.rs", "pub struct «ExactStartMatch»;\n", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "query": "Exact" }
    });
    let res = handle_workspace_symbol(&req, [&idx].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "ExactStartMatch");
}
