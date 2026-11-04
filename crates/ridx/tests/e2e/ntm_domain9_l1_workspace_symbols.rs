/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Hermetic integration tests for Domain 9: workspace/symbol and workspaceSymbol/resolve.

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
fn test_l1_workspace_symbol_empty_query_lists_all() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_ws_query", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_ws_query", "src/lib.rs", "pub fn «alpha»() {}\npub fn «beta»() {}\npub fn «gamma»() {}\n", DefKind::Fn).unwrap();
    assert_eq!(ids.len(), 3);

    let shard = ws.build_crate_index("pkg_ws_query").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let mut conv = PositionConverter::new();

    let req = make_ws_query("");
    let res = handle_workspace_symbol(&req, std::iter::once(&index), &mut conv);
    let items = res.as_array().expect("symbol list");

    assert_eq!(items.len(), 3, "Empty query must return all indexed symbols");
    let names: Vec<&str> = items.iter().filter_map(|s| s["name"].as_str()).collect();
    assert_eq!(names, vec!["alpha", "beta", "gamma"]);

    let expected_uri = path_to_uri(ws.file_path("pkg_ws_query", "src/lib.rs").unwrap().to_str().unwrap());

    // alpha
    assert_eq!(items[0]["name"], "alpha");
    assert_eq!(items[0]["kind"], 12);
    assert_eq!(items[0]["containerName"], "pkg_ws_query");
    assert_eq!(items[0]["location"]["uri"], expected_uri);
    assert_eq!(items[0]["location"]["range"]["start"]["line"], 0);
    assert_eq!(items[0]["location"]["range"]["start"]["character"], 7);
    assert_eq!(items[0]["location"]["range"]["end"]["line"], 0);
    assert_eq!(items[0]["location"]["range"]["end"]["character"], 12);

    // beta
    assert_eq!(items[1]["name"], "beta");
    assert_eq!(items[1]["kind"], 12);
    assert_eq!(items[1]["containerName"], "pkg_ws_query");
    assert_eq!(items[1]["location"]["uri"], expected_uri);
    assert_eq!(items[1]["location"]["range"]["start"]["line"], 1);
    assert_eq!(items[1]["location"]["range"]["start"]["character"], 7);
    assert_eq!(items[1]["location"]["range"]["end"]["line"], 1);
    assert_eq!(items[1]["location"]["range"]["end"]["character"], 11);

    // gamma
    assert_eq!(items[2]["name"], "gamma");
    assert_eq!(items[2]["kind"], 12);
    assert_eq!(items[2]["containerName"], "pkg_ws_query");
    assert_eq!(items[2]["location"]["uri"], expected_uri);
    assert_eq!(items[2]["location"]["range"]["start"]["line"], 2);
    assert_eq!(items[2]["location"]["range"]["start"]["character"], 7);
    assert_eq!(items[2]["location"]["range"]["end"]["line"], 2);
    assert_eq!(items[2]["location"]["range"]["end"]["character"], 12);
}

#[test]
fn test_l1_workspace_symbol_exact_match() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_exact", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_exact", "src/lib.rs", "pub fn «compute_hash»() {}\npub fn «compute_other»() {}\n", DefKind::Fn).unwrap();

    let shard = ws.build_crate_index("pkg_exact").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let mut conv = PositionConverter::new();

    let req = make_ws_query("compute_hash");
    let res = handle_workspace_symbol(&req, std::iter::once(&index), &mut conv);
    let items = res.as_array().expect("symbol list");

    assert_eq!(items.len(), 1);
    let exact = &items[0];
    assert_eq!(exact["name"], "compute_hash");
    assert_eq!(exact["location"]["range"]["start"]["line"], 0);
    assert_eq!(exact["location"]["range"]["start"]["character"], 7);
    assert_eq!(exact["location"]["range"]["end"]["line"], 0);
    assert_eq!(exact["location"]["range"]["end"]["character"], 19);
}

#[test]
fn test_l1_workspace_symbol_case_insensitive() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_case", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_case", "src/types.rs", "pub struct «Processor»;\n", DefKind::Struct).unwrap();
    ws.add_source_file_with_defs("pkg_case", "src/funcs.rs", "pub fn «process_task»() {}\n", DefKind::Fn).unwrap();

    let shard = ws.build_crate_index("pkg_case").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let mut conv = PositionConverter::new();

    // A lowercase query matches case-insensitively. As in rust-analyzer, types are searched first: the struct
    // matches, so the function is not searched.
    let req = make_ws_query("process");
    let res = handle_workspace_symbol(&req, std::iter::once(&index), &mut conv);
    let items = res.as_array().expect("symbol list");

    assert_eq!(items.len(), 1, "Types come first: Processor only");
    let proc_struct = items.iter().find(|s| s["name"] == "Processor").unwrap();
    assert_eq!(proc_struct["kind"], 23); // Struct
    assert_eq!(proc_struct["location"]["range"]["start"]["character"], 11);

    // No type matches `process_t`, so every kind is searched and the function is found.
    let res_fn = handle_workspace_symbol(&make_ws_query("process_t"), std::iter::once(&index), &mut conv);
    let items_fn = res_fn.as_array().expect("symbol list");
    let proc_fn = items_fn.iter().find(|s| s["name"] == "process_task").unwrap();
    assert_eq!(proc_fn["kind"], 12); // Function
    assert_eq!(proc_fn["location"]["range"]["start"]["character"], 7);
}

#[test]
fn test_l1_workspace_symbol_kind_mapping() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_kinds", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_kinds", "src/structs.rs", "pub struct «Config»;\n", DefKind::Struct).unwrap();
    ws.add_source_file_with_defs("pkg_kinds", "src/enums.rs", "pub enum «Status» {}\n", DefKind::Enum).unwrap();
    ws.add_source_file_with_defs("pkg_kinds", "src/traits.rs", "pub trait «Worker» {}\n", DefKind::Trait).unwrap();
    ws.add_source_file_with_defs("pkg_kinds", "src/consts.rs", "pub const «MAX_COUNT»: u32 = 10;\n", DefKind::Const).unwrap();

    let shard = ws.build_crate_index("pkg_kinds").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let mut conv = PositionConverter::new();

    let req = make_ws_query("");
    let res = handle_workspace_symbol(&req, std::iter::once(&index), &mut conv);
    let items = res.as_array().expect("symbol list");

    assert_eq!(items.len(), 4);

    let cfg = items.iter().find(|s| s["name"] == "Config").unwrap();
    assert_eq!(cfg["kind"], 23); // Struct
    assert_eq!(cfg["location"]["range"]["start"]["character"], 11);

    let status = items.iter().find(|s| s["name"] == "Status").unwrap();
    assert_eq!(status["kind"], 10); // Enum
    assert_eq!(status["location"]["range"]["start"]["character"], 9);

    let worker = items.iter().find(|s| s["name"] == "Worker").unwrap();
    assert_eq!(worker["kind"], 11); // Interface / Trait
    assert_eq!(worker["location"]["range"]["start"]["character"], 10);

    let max_c = items.iter().find(|s| s["name"] == "MAX_COUNT").unwrap();
    assert_eq!(max_c["kind"], 14); // Constant
    assert_eq!(max_c["location"]["range"]["start"]["character"], 10);
}

#[test]
fn test_l1_workspace_symbol_same_index_twice_deduplicated() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_dedup", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_dedup", "src/lib.rs", "pub fn «unique_task»() {}\n", DefKind::Fn).unwrap();

    let shard = ws.build_crate_index("pkg_dedup").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let mut conv = PositionConverter::new();

    let req = make_ws_query("unique_task");
    // Pass the same index twice to simulate overlapping roots
    let res = handle_workspace_symbol(&req, [&index, &index].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");

    assert_eq!(items.len(), 1, "Duplicate occurrences across workspace roots must be deduplicated");
    assert_eq!(items[0]["name"], "unique_task");
}

#[test]
fn test_l1_workspace_symbol_no_matches_returns_empty() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_empty", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_empty", "src/lib.rs", "pub fn «existing»() {}\n", DefKind::Fn).unwrap();

    let shard = ws.build_crate_index("pkg_empty").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let mut conv = PositionConverter::new();

    let req = make_ws_query("ghost_symbol_never_exists");
    let res = handle_workspace_symbol(&req, std::iter::once(&index), &mut conv);
    assert_eq!(res, json!([]));
}

#[test]
fn test_l1_workspace_symbol_resolve_missing_location() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_resolve", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_resolve", "src/lib.rs", "pub struct «TargetRecord»;\n", DefKind::Struct).unwrap();

    let shard = ws.build_crate_index("pkg_resolve").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let mut conv = PositionConverter::new();

    // Incomplete symbol item without location
    let unresolved_req = json!({
        "params": {
            "name": "TargetRecord",
            "kind": 23
        }
    });

    let expected_uri = path_to_uri(ws.file_path("pkg_resolve", "src/lib.rs").unwrap().to_str().unwrap());

    let resolved = handle_workspace_symbol_resolve(&unresolved_req, std::iter::once(&index), &mut conv);
    assert_eq!(resolved["name"], "TargetRecord");
    assert_eq!(resolved["kind"], 23);
    assert_eq!(resolved["containerName"], "pkg_resolve");
    assert_eq!(resolved["location"]["uri"], expected_uri);
    assert_eq!(resolved["location"]["range"]["start"]["line"], 0);
    assert_eq!(resolved["location"]["range"]["start"]["character"], 11);
    assert_eq!(resolved["location"]["range"]["end"]["line"], 0);
    assert_eq!(resolved["location"]["range"]["end"]["character"], 23);
}

#[test]
fn test_l1_workspace_symbol_resolve_nonexistent_returns_as_is() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_unknown", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_unknown", "src/lib.rs", "pub fn «present»() {}\n", DefKind::Fn).unwrap();

    let shard = ws.build_crate_index("pkg_unknown").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let mut conv = PositionConverter::new();

    let unresolved_req = json!({
        "params": {
            "name": "MissingSymbol",
            "kind": 12
        }
    });

    let resolved = handle_workspace_symbol_resolve(&unresolved_req, std::iter::once(&index), &mut conv);
    assert_eq!(resolved, json!({"name": "MissingSymbol", "kind": 12}));
}

#[test]
fn test_l1_workspace_symbol_resolve_null_params_safe() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_null", "0.1.0").unwrap();
    let shard = ws.build_crate_index("pkg_null").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let mut conv = PositionConverter::new();

    let null_req = json!({ "params": null });
    let res = handle_workspace_symbol_resolve(&null_req, std::iter::once(&index), &mut conv);
    assert_eq!(res, Value::Null);
}
