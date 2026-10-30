/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspaceSymbol/resolve (Part 3):
//! preservation of kind on location substitution, explicit location: null handling,
//! multi-crate same-name candidate shard order, and non-ASCII symbol names.

use ridx::lsp::protocol::PositionConverter;
use ridx::lsp::symbols::handle_workspace_symbol_resolve;
use ridx_format::{DefKind, WorkspaceIndex};
use ridx_test_support::fixture::SyntheticWorkspaceFixture;
use serde_json::{json, Value};

fn assert_range(loc: &Value, line: u32, start_col: u32, end_col: u32) {
    assert_eq!(loc["range"]["start"]["line"], line);
    assert_eq!(loc["range"]["start"]["character"], start_col);
    assert_eq!(loc["range"]["end"]["line"], line);
    assert_eq!(loc["range"]["end"]["character"], end_col);
}

#[test]
fn test_l1_workspace_symbol_resolve_preserves_kind_with_location_sub() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_resolve_kind", "0.1.0").unwrap();

    let (_, ids) = ws.add_source_file_with_defs("pkg_resolve_kind", "src/lib.rs", "pub struct «NodeWorker»;\n", DefKind::Struct).unwrap();
    assert_eq!(ids.len(), 1);

    let shard = ws.build_crate_index("pkg_resolve_kind").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_resolve_kind", "src/lib.rs").unwrap();

    // Query params has name and explicit kind: 23 (Struct), but no location
    let req = json!({
        "params": {
            "name": "NodeWorker",
            "kind": 23
        }
    });

    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&index), &mut conv);
    assert_eq!(res["name"], "NodeWorker");
    assert_eq!(res["kind"], 23, "Symbol kind must be preserved when location is substituted");
    assert_eq!(res["containerName"], "pkg_resolve_kind");
    assert_eq!(res["location"]["uri"], uri);
    assert_range(&res["location"], 0, 11, 21);
}

#[test]
fn test_l1_workspace_symbol_resolve_explicit_null_location_substitutes() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_resolve_null", "0.1.0").unwrap();

    let (_, ids) = ws.add_source_file_with_defs("pkg_resolve_null", "src/lib.rs", "pub struct «NullTarget»;\n", DefKind::Struct).unwrap();
    assert_eq!(ids.len(), 1);

    let shard = ws.build_crate_index("pkg_resolve_null").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_resolve_null", "src/lib.rs").unwrap();

    // Query params has explicit location: null (not absent key)
    let req = json!({
        "params": {
            "name": "NullTarget",
            "kind": 23,
            "location": Value::Null
        }
    });

    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&index), &mut conv);
    assert_eq!(res["name"], "NullTarget");
    assert_eq!(res["kind"], 23);
    assert_eq!(res["containerName"], "pkg_resolve_null");
    assert_eq!(res["location"]["uri"], uri);
    assert_range(&res["location"], 0, 11, 21);
}

#[test]
fn test_l1_workspace_symbol_resolve_same_name_multiple_crates_first_shard() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("crate_one", "0.1.0").unwrap();
    ws.add_crate("crate_two", "0.1.0").unwrap();

    let (_, ids_one) = ws.add_source_file_with_defs("crate_one", "src/lib.rs", "pub struct «SharedConfig»;\n", DefKind::Struct).unwrap();
    assert_eq!(ids_one.len(), 1);

    let (_, ids_two) = ws.add_source_file_with_defs("crate_two", "src/lib.rs", "pub struct «SharedConfig»;\n", DefKind::Struct).unwrap();
    assert_eq!(ids_two.len(), 1);

    let shard_one = ws.build_crate_index("crate_one").unwrap();
    let shard_two = ws.build_crate_index("crate_two").unwrap();

    let idx_one = WorkspaceIndex::open(&shard_one).unwrap();
    let idx_two = WorkspaceIndex::open(&shard_two).unwrap();
    let mut conv = PositionConverter::new();
    let uri_one = ws.file_uri("crate_one", "src/lib.rs").unwrap();
    let uri_two = ws.file_uri("crate_two", "src/lib.rs").unwrap();

    let req = json!({
        "params": {
            "name": "SharedConfig"
        }
    });

    // 1. Order [idx_one, idx_two] resolves to crate_one
    let workspaces_first = vec![&idx_one, &idx_two];
    let res_first = handle_workspace_symbol_resolve(&req, workspaces_first.into_iter(), &mut conv);
    assert_eq!(res_first["name"], "SharedConfig");
    assert_eq!(res_first["containerName"], "crate_one", "Must pick first shard in iteration order");
    assert_eq!(res_first["location"]["uri"], uri_one);

    // 2. Order [idx_two, idx_one] resolves to crate_two
    let workspaces_second = vec![&idx_two, &idx_one];
    let res_second = handle_workspace_symbol_resolve(&req, workspaces_second.into_iter(), &mut conv);
    assert_eq!(res_second["name"], "SharedConfig");
    assert_eq!(res_second["containerName"], "crate_two", "Must pick first shard in reversed iteration order");
    assert_eq!(res_second["location"]["uri"], uri_two);
}

#[test]
fn test_l1_workspace_symbol_resolve_non_ascii_symbol_name() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_non_ascii", "0.1.0").unwrap();

    // Symbol with Greek non-ASCII character (\u{03B4} = δ)
    let (_, ids) = ws.add_source_file_with_defs("pkg_non_ascii", "src/lib.rs", "pub fn «delta_\u{03B4}»() {}\n", DefKind::Fn).unwrap();
    assert_eq!(ids.len(), 1);

    let shard = ws.build_crate_index("pkg_non_ascii").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_non_ascii", "src/lib.rs").unwrap();

    let req = json!({
        "params": {
            "name": "delta_\u{03B4}"
        }
    });

    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&index), &mut conv);
    assert_eq!(res["name"], "delta_\u{03B4}");
    assert_eq!(res["containerName"], "pkg_non_ascii");
    assert_eq!(res["location"]["uri"], uri);
    assert_range(&res["location"], 0, 7, 14);
}
