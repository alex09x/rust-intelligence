/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Hermetic integration tests for Domain 9: workspace/symbol and resolve edge cases (Part 2).

use ridx::lsp::protocol::{path_to_uri, PositionConverter};
use ridx::lsp::symbols::{handle_workspace_symbol, handle_workspace_symbol_resolve};
use ridx_format::{DefKind, WorkspaceIndex};
use ridx_test_support::fixture::SyntheticWorkspaceFixture;
use serde_json::{json, Value};

fn make_ws_query(query: &str) -> Value {
    json!({
        "params": {
            "query": query
        }
    })
}

#[test]
fn test_l1_workspace_symbol_cross_crate_same_name() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("crate_alpha", "0.1.0").unwrap();
    ws.add_source_file_with_defs("crate_alpha", "src/lib.rs", "pub fn «shared_function»() {}\n", DefKind::Fn).unwrap();

    ws.add_crate("crate_beta", "0.1.0").unwrap();
    ws.add_source_file_with_defs("crate_beta", "src/lib.rs", "pub fn «shared_function»() {}\n", DefKind::Fn).unwrap();

    let shard_a = ws.build_crate_index("crate_alpha").unwrap();
    let index_a = WorkspaceIndex::open(&shard_a).unwrap();
    let shard_b = ws.build_crate_index("crate_beta").unwrap();
    let index_b = WorkspaceIndex::open(&shard_b).unwrap();
    let mut conv = PositionConverter::new();

    let req = make_ws_query("shared_function");
    let res = handle_workspace_symbol(&req, [&index_a, &index_b].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");

    assert_eq!(items.len(), 2, "Must return matches from both crates");
    assert_eq!(items[0]["name"], "shared_function");
    assert_eq!(items[0]["containerName"], "crate_alpha");
    assert_eq!(items[1]["name"], "shared_function");
    assert_eq!(items[1]["containerName"], "crate_beta");

    let uri_a = path_to_uri(ws.file_path("crate_alpha", "src/lib.rs").unwrap().to_str().unwrap());
    let uri_b = path_to_uri(ws.file_path("crate_beta", "src/lib.rs").unwrap().to_str().unwrap());
    assert_eq!(items[0]["location"]["uri"], uri_a);
    assert_eq!(items[0]["location"]["range"]["start"]["line"], 0);
    assert_eq!(items[0]["location"]["range"]["start"]["character"], 7);
    assert_eq!(items[0]["location"]["range"]["end"]["character"], 22);
    assert_eq!(items[1]["location"]["uri"], uri_b);
    assert_eq!(items[1]["location"]["range"]["start"]["line"], 0);
    assert_eq!(items[1]["location"]["range"]["start"]["character"], 7);
    assert_eq!(items[1]["location"]["range"]["end"]["character"], 22);
}

#[test]
fn test_l1_workspace_symbol_cap_128_across_shards() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("crate_first", "0.1.0").unwrap();
    let mut code_first = String::new();
    for i in 0..80 {
        code_first.push_str(&format!("pub fn «item_{i:02}»() {{}}\n"));
    }
    ws.add_source_file_with_defs("crate_first", "src/lib.rs", &code_first, DefKind::Fn).unwrap();

    ws.add_crate("crate_second", "0.1.0").unwrap();
    let mut code_second = String::new();
    for i in 0..80 {
        code_second.push_str(&format!("pub fn «item_{i:02}»() {{}}\n"));
    }
    ws.add_source_file_with_defs("crate_second", "src/lib.rs", &code_second, DefKind::Fn).unwrap();

    let shard_1 = ws.build_crate_index("crate_first").unwrap();
    let index_1 = WorkspaceIndex::open(&shard_1).unwrap();
    let shard_2 = ws.build_crate_index("crate_second").unwrap();
    let index_2 = WorkspaceIndex::open(&shard_2).unwrap();
    let mut conv = PositionConverter::new();

    let req = make_ws_query("");
    let res = handle_workspace_symbol(&req, [&index_1, &index_2].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");

    // Total is capped at rust-analyzer's default limit of 128.
    assert_eq!(items.len(), 128, "Workspace symbols must cap at exactly 128");
    let count_first = items.iter().filter(|s| s["containerName"] == "crate_first").count();
    let count_second = items.iter().filter(|s| s["containerName"] == "crate_second").count();
    assert_eq!(count_first, 80, "First crate definitions must be exhausted");
    assert_eq!(count_second, 48, "Second crate must provide remaining 48 to cap");
}

#[test]
fn test_l1_workspace_symbol_path_matching() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_path_match", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_path_match", "src/actions.rs", "pub fn «action_handler»() {}\npub fn «action_fallback»() {}\n", DefKind::Fn).unwrap();

    let shard = ws.build_crate_index("pkg_path_match").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let mut conv = PositionConverter::new();

    // A query naming the crate finds the crate itself, as in rust-analyzer, not every item in it.
    let req_crate = make_ws_query("pkg_path_match");
    let res_crate = handle_workspace_symbol(&req_crate, std::iter::once(&index), &mut conv);
    let items_crate = res_crate.as_array().expect("items list");
    let names: Vec<&str> = items_crate.iter().filter_map(|s| s["name"].as_str()).collect();
    assert_eq!(names, vec!["pkg_path_match"]);

    // Query with qualified path separator (matches only via path)
    let req_path = make_ws_query("::action_f");
    let res_path = handle_workspace_symbol(&req_path, std::iter::once(&index), &mut conv);
    let items_path = res_path.as_array().expect("items list");
    assert_eq!(items_path.len(), 1);
    assert_eq!(items_path[0]["name"], "action_fallback");
    assert_eq!(items_path[0]["location"]["range"]["start"]["line"], 1);
    assert_eq!(items_path[0]["location"]["range"]["start"]["character"], 7);
    assert_eq!(items_path[0]["location"]["range"]["end"]["character"], 22);
}

#[test]
fn test_l1_workspace_symbol_non_ascii_german_query() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_german", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_german", "src/lib.rs", "pub fn «grüßen»() {}\npub fn «groß»() {}\n", DefKind::Fn).unwrap();

    let shard = ws.build_crate_index("pkg_german").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let mut conv = PositionConverter::new();

    let req = make_ws_query("grüß");
    let res = handle_workspace_symbol(&req, std::iter::once(&index), &mut conv);
    let items = res.as_array().expect("symbol list");

    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "grüßen");
    assert_eq!(items[0]["location"]["range"]["start"]["line"], 0);
    assert_eq!(items[0]["location"]["range"]["start"]["character"], 7);
    assert_eq!(items[0]["location"]["range"]["end"]["character"], 13);
}

#[test]
fn test_l1_workspace_symbol_utf16_location_with_emoji_prefix() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_crab", "0.1.0").unwrap();
    // 2 crabs in comment: each crab takes 2 UTF-16 code units (surrogate pair)
    // "/* 🦀🦀 */ pub fn " = 3 + 4 + 4 + 7 = 18 UTF-16 code units
    ws.add_source_file_with_defs("pkg_crab", "src/lib.rs", "/* 🦀🦀 */ pub fn «crab_handler»() {}\n", DefKind::Fn).unwrap();

    let shard = ws.build_crate_index("pkg_crab").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let mut conv = PositionConverter::new();

    let req = make_ws_query("crab_handler");
    let res = handle_workspace_symbol(&req, std::iter::once(&index), &mut conv);
    let items = res.as_array().expect("symbol list");

    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "crab_handler");
    assert_eq!(items[0]["location"]["range"]["start"]["line"], 0);
    // 3 ("/* ") + 4 (two surrogate pairs) + 4 (" */ ") + 7 ("pub fn ") = 18
    assert_eq!(items[0]["location"]["range"]["start"]["character"], 18);
    assert_eq!(items[0]["location"]["range"]["end"]["character"], 30);
}

#[test]
fn test_l1_workspace_symbol_resolve_already_populated() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_pop", "0.1.0").unwrap();
    let shard = ws.build_crate_index("pkg_pop").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let mut conv = PositionConverter::new();

    let already_resolved = json!({
        "params": {
            "name": "PopulatedRecord",
            "kind": 23,
            "containerName": "pkg_pop",
            "location": {
                "uri": "file:///workspace/src/lib.rs",
                "range": {
                    "start": { "line": 5, "character": 4 },
                    "end": { "line": 5, "character": 19 }
                }
            }
        }
    });

    let res = handle_workspace_symbol_resolve(&already_resolved, std::iter::once(&index), &mut conv);
    assert_eq!(res, already_resolved["params"]);
}

#[test]
fn test_l1_workspace_symbol_resolve_exact_match_priority() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_priority", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_priority", "src/lib.rs", "pub fn «task_runner»() {}\npub fn «task»() {}\n", DefKind::Fn).unwrap();

    let shard = ws.build_crate_index("pkg_priority").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let mut conv = PositionConverter::new();

    // Query resolve for exact "task" when "task_runner" also exists in shard
    let req = json!({
        "params": {
            "name": "task",
            "kind": 12
        }
    });

    let resolved = handle_workspace_symbol_resolve(&req, std::iter::once(&index), &mut conv);
    assert_eq!(resolved["name"], "task");
    assert_eq!(resolved["kind"], 12);
    // "task" is on line 1, not line 0
    assert_eq!(resolved["location"]["range"]["start"]["line"], 1);
    assert_eq!(resolved["location"]["range"]["start"]["character"], 7);
    assert_eq!(resolved["location"]["range"]["end"]["character"], 11);
}

#[test]
fn test_l1_workspace_symbol_typaram_excluded() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_typaram", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_typaram", "src/lib.rs", "pub fn «run_generic»() {}\n", DefKind::Fn).unwrap();
    ws.add_source_file_with_defs("pkg_typaram", "src/types.rs", "pub struct Holder<«T»>(T);\n", DefKind::TyParam).unwrap();

    let shard = ws.build_crate_index("pkg_typaram").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let mut conv = PositionConverter::new();

    let req = make_ws_query("");
    let res = handle_workspace_symbol(&req, std::iter::once(&index), &mut conv);
    let items = res.as_array().expect("symbol list");

    // TyParam should be excluded by kind filter in search_symbols
    assert_eq!(items.len(), 1, "TyParam must be excluded from workspace symbol results");
    assert_eq!(items[0]["name"], "run_generic");
}
