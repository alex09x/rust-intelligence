/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use ridx::lsp::protocol::PositionConverter;
use ridx::lsp::symbols::handle_workspace_symbol;
use ridx_format::{DefKind, WorkspaceIndex};
use ridx_test_support::fixture::SyntheticWorkspaceFixture;
use serde_json::{json, Value};

fn assert_range(range: &Value, start_line: u32, start_char: u32, end_line: u32, end_char: u32) {
    assert_eq!(range["start"]["line"], start_line);
    assert_eq!(range["start"]["character"], start_char);
    assert_eq!(range["end"]["line"], end_line);
    assert_eq!(range["end"]["character"], end_char);
}

#[test]
fn test_l1_workspace_symbol_multiple_indices_merged_in_order_and_deduplicated() {
    let mut ws_a = SyntheticWorkspaceFixture::new().unwrap();
    ws_a.add_crate("crate_a", "0.1.0").unwrap();
    ws_a.add_source_file_with_defs("crate_a", "src/lib.rs", "pub struct «AlphaStruct»;\n", DefKind::Struct).unwrap();
    ws_a.build_all_indices().unwrap();
    let idx_a = WorkspaceIndex::open(ws_a.root_path()).unwrap();

    let mut ws_b = SyntheticWorkspaceFixture::new().unwrap();
    ws_b.add_crate("crate_b", "0.1.0").unwrap();
    ws_b.add_source_file_with_defs("crate_b", "src/lib.rs", "pub struct «BetaStruct»;\n", DefKind::Struct).unwrap();
    ws_b.build_all_indices().unwrap();
    let idx_b = WorkspaceIndex::open(ws_b.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": {
            "query": ""
        }
    });

    // Request 1: [&idx_a, &idx_b, &idx_a]
    // Verifies symbols from multiple index objects are merged, deduplicated, and preserve iteration order
    let res1 = handle_workspace_symbol(&req, [&idx_a, &idx_b, &idx_a].into_iter(), &mut conv);
    let items1 = res1.as_array().expect("array of symbols");
    assert_eq!(items1.len(), 2);
    assert_eq!(items1[0]["name"], "AlphaStruct");
    assert_eq!(items1[1]["name"], "BetaStruct");
    assert_range(&items1[0]["location"]["range"], 0, 11, 0, 22);
    assert_range(&items1[1]["location"]["range"], 0, 11, 0, 21);

    // Request 2: [&idx_b, &idx_a]
    // Iteration starts with idx_b, so BetaStruct appears first
    let res2 = handle_workspace_symbol(&req, [&idx_b, &idx_a].into_iter(), &mut conv);
    let items2 = res2.as_array().expect("array of symbols");
    assert_eq!(items2.len(), 2);
    assert_eq!(items2[0]["name"], "BetaStruct");
    assert_eq!(items2[1]["name"], "AlphaStruct");
}

#[test]
fn test_l1_workspace_symbol_missing_or_null_query_param_defaults_to_all() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_defaults", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_defaults", "src/lib.rs", "pub struct «FirstItem»;\npub struct «SecondItem»;\n", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let mut conv = PositionConverter::new();

    // 1. Missing "query" key completely in params
    let req_missing = json!({
        "params": {}
    });
    let res_missing = handle_workspace_symbol(&req_missing, std::iter::once(&index), &mut conv);
    let items_missing = res_missing.as_array().expect("array of symbols");
    let mut names_missing: Vec<&str> = items_missing.iter().map(|s| s["name"].as_str().unwrap()).collect();
    names_missing.sort();
    assert_eq!(names_missing, vec!["FirstItem", "SecondItem"]);

    // 2. Explicit null "query" key in params
    let req_null = json!({
        "params": {
            "query": null
        }
    });
    let res_null = handle_workspace_symbol(&req_null, std::iter::once(&index), &mut conv);
    let items_null = res_null.as_array().expect("array of symbols");
    assert_eq!(items_null.len(), 2);
    let mut names_null: Vec<&str> = items_null.iter().map(|s| s["name"].as_str().unwrap()).collect();
    names_null.sort();
    assert_eq!(names_null, vec!["FirstItem", "SecondItem"]);
}

#[test]
fn test_l1_workspace_symbol_deep_module_hierarchy_container_name() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_deep", "0.1.0").unwrap();
    ws.add_source_file_with_prefix_defs("pkg_deep", "src/deep.rs", "pub struct «DeepStruct»;\n", "pkg_deep::sub_a::sub_b::sub_c", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let mut conv = PositionConverter::new();

    let req = json!({
        "params": {
            "query": "DeepStruct"
        }
    });
    let res = handle_workspace_symbol(&req, std::iter::once(&index), &mut conv);
    let items = res.as_array().expect("array of symbols");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "DeepStruct");
    // containerName is extracted via s.path.rsplit("::").nth(1), returning immediate parent "sub_c"
    assert_eq!(items[0]["containerName"], "sub_c");
    assert_range(&items[0]["location"]["range"], 0, 11, 0, 21);
}

#[test]
fn test_l1_workspace_symbol_submodule_path_query_matching() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_submod", "0.1.0").unwrap();
    ws.add_source_file_with_prefix_defs("pkg_submod", "src/proto.rs", "pub struct «Message»;\n", "pkg_submod::proto", DefKind::Struct).unwrap();
    ws.add_source_file_with_prefix_defs("pkg_submod", "src/other.rs", "pub struct «Unrelated»;\n", "pkg_submod::other", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let mut conv = PositionConverter::new();

    // Query for "proto::" matches symbol path "pkg_submod::proto::Message"
    let req = json!({
        "params": {
            "query": "proto::"
        }
    });
    let res = handle_workspace_symbol(&req, std::iter::once(&index), &mut conv);
    let items = res.as_array().expect("array of symbols");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Message");
    assert_eq!(items[0]["containerName"], "proto");
    assert_range(&items[0]["location"]["range"], 0, 11, 0, 18);
}

#[test]
fn test_l1_workspace_symbol_whitespace_padded_query_returns_empty() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_ws", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_ws", "src/lib.rs", "pub struct «MyPaddedItem»;\n", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let mut conv = PositionConverter::new();

    // 1. Control query without whitespace matches exactly
    let req_clean = json!({
        "params": {
            "query": "MyPaddedItem"
        }
    });
    let res_clean = handle_workspace_symbol(&req_clean, std::iter::once(&index), &mut conv);
    let items_clean = res_clean.as_array().expect("array of symbols");
    assert_eq!(items_clean.len(), 1);
    assert_eq!(items_clean[0]["name"], "MyPaddedItem");
    assert_range(&items_clean[0]["location"]["range"], 0, 11, 0, 23);

    // 2. Query padded with leading/trailing spaces matches literally and returns empty
    let req_padded = json!({
        "params": {
            "query": "  MyPaddedItem  "
        }
    });
    let res_padded = handle_workspace_symbol(&req_padded, std::iter::once(&index), &mut conv);
    assert_eq!(res_padded, json!([]));
}

#[test]
fn test_l1_workspace_symbol_cap_100_single_shard_boundary() {
    let mut ws_100 = SyntheticWorkspaceFixture::new().unwrap();
    ws_100.add_crate("pkg_cap_100", "0.1.0").unwrap();

    // Register exactly 100 definitions in a single shard
    let mut code_100 = String::new();
    for i in 0..100 {
        code_100.push_str(&format!("pub struct «Sym{:03}»;\n", i));
    }
    ws_100.add_source_file_with_defs("pkg_cap_100", "src/lib.rs", &code_100, DefKind::Struct).unwrap();
    ws_100.build_all_indices().unwrap();

    let index_100 = WorkspaceIndex::open(ws_100.root_path()).unwrap();
    let mut conv = PositionConverter::new();

    let req = json!({
        "params": {
            "query": ""
        }
    });

    // Exactly 100 symbols in single shard -> returns 100
    let res_100 = handle_workspace_symbol(&req, std::iter::once(&index_100), &mut conv);
    let items_100 = res_100.as_array().expect("array of symbols");
    assert_eq!(items_100.len(), 100);

    // Register exactly 101 definitions in an isolated single shard
    let mut ws_101 = SyntheticWorkspaceFixture::new().unwrap();
    ws_101.add_crate("pkg_cap_101", "0.1.0").unwrap();
    let mut code_101 = String::new();
    for i in 0..101 {
        code_101.push_str(&format!("pub struct «Sym{:03}»;\n", i));
    }
    ws_101.add_source_file_with_defs("pkg_cap_101", "src/lib.rs", &code_101, DefKind::Struct).unwrap();
    ws_101.build_all_indices().unwrap();

    let index_101 = WorkspaceIndex::open(ws_101.root_path()).unwrap();
    // 101 symbols in single shard -> inner loop breaks at results.len() >= 100 -> returns 100
    let res_101 = handle_workspace_symbol(&req, std::iter::once(&index_101), &mut conv);
    let items_101 = res_101.as_array().expect("array of symbols");
    assert_eq!(items_101.len(), 100);
}
