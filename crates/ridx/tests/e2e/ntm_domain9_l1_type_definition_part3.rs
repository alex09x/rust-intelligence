/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/typeDefinition (Part 3):
//! cross-crate multi-shard, same-named types preferring local crate,
//! qualified path suffix and exact matching, UTF-16 query-side column conversion,
//! and generic wrapper type string safe rejection.

use ridx::lsp::navigation::handle_type_definition;
use ridx::lsp::protocol::PositionConverter;
use ridx::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use ridx_test_support::fixture::SyntheticWorkspaceFixture;
use serde_json::{json, Value};

fn make_pos_req(uri: &str, line: u32, col: u32) -> Value {
    json!({
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": col }
        }
    })
}

fn assert_range(range: &Value, start_line: u32, start_col: u32, end_line: u32, end_col: u32) {
    assert_eq!(range["start"]["line"], start_line);
    assert_eq!(range["start"]["character"], start_col);
    assert_eq!(range["end"]["line"], end_line);
    assert_eq!(range["end"]["character"], end_col);
}

#[test]
fn test_l1_type_definition_cross_crate_two_shards() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("types_crate", "0.1.0").unwrap();
    ws.add_crate("client_crate", "0.1.0").unwrap();

    let (_, type_ids) = ws.add_source_file_with_defs("types_crate", "src/lib.rs", "pub struct «RemoteConfig»;\n", DefKind::Struct).unwrap();
    assert_eq!(type_ids.len(), 1);

    ws.add_source_file("client_crate", "src/lib.rs", "pub fn setup(cfg: RemoteConfig) {}\n").unwrap();
    // Line 1, 1-based cols 14..17 is `cfg`, type is "RemoteConfig"
    ws.add_type_at("client_crate", "src/lib.rs", 1, 14, 1, 17, "RemoteConfig").unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    assert_eq!(index.readers().len(), 2);

    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let client_uri = ws.file_uri("client_crate", "src/lib.rs").unwrap();
    let types_uri = ws.file_uri("types_crate", "src/lib.rs").unwrap();

    let req = make_pos_req(&client_uri, 0, 14);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);

    assert_eq!(res["uri"], types_uri);
    assert_range(&res["range"], 0, 11, 0, 23);
}

#[test]
fn test_l1_type_definition_same_named_type_prefers_local_crate() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    // aaa_shared is first shard, zzz_local is second shard
    ws.add_crate("aaa_shared", "0.1.0").unwrap();
    ws.add_crate("zzz_local", "0.1.0").unwrap();

    // Both crates define `Record` in their respective `src/lib.rs`
    ws.add_source_file_with_defs("aaa_shared", "src/lib.rs", "pub struct «Record»;\n", DefKind::Struct).unwrap();
    ws.add_source_file_with_defs("zzz_local", "src/lib.rs", "pub struct «Record»;\n", DefKind::Struct).unwrap();

    // Query is made from a DIFFERENT file in zzz_local (`src/main.rs`), ensuring same-file tier does not match
    let main_code = "pub fn main() { let r = 42; }\n";
    ws.add_source_file("zzz_local", "src/main.rs", main_code).unwrap();
    // Line 1, 1-based cols 21..22 is `r`, type is "Record"
    ws.add_type_at("zzz_local", "src/main.rs", 1, 21, 1, 22, "Record").unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    assert_eq!(index.readers().len(), 2);

    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let main_uri = ws.file_uri("zzz_local", "src/main.rs").unwrap();
    let local_lib_uri = ws.file_uri("zzz_local", "src/lib.rs").unwrap();

    let req = make_pos_req(&main_uri, 0, 20);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);

    // in_same_crate selects zzz_local/src/lib.rs over aaa_shared/src/lib.rs
    assert_eq!(res["uri"], local_lib_uri);
    assert_range(&res["range"], 0, 11, 0, 17);
}

#[test]
fn test_l1_type_definition_qualified_path_suffix_fallback() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("driver_pkg", "0.1.0").unwrap();

    // Definition in src/service.rs with path driver_pkg::service::DriverHandle
    let (_, ids) = ws.add_source_file_with_prefix_defs("driver_pkg", "src/service.rs", "pub struct «DriverHandle»;\n", "driver_pkg::service", DefKind::Struct).unwrap();
    assert_eq!(ids.len(), 1);

    // Query file has type_at registering partially qualified suffix "service::DriverHandle"
    let code = "pub fn get() { let h = 1; }\n";
    ws.add_source_file("driver_pkg", "src/lib.rs", code).unwrap();
    ws.add_type_at("driver_pkg", "src/lib.rs", 1, 20, 1, 21, "service::DriverHandle").unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let lib_uri = ws.file_uri("driver_pkg", "src/lib.rs").unwrap();
    let service_uri = ws.file_uri("driver_pkg", "src/service.rs").unwrap();

    let req = make_pos_req(&lib_uri, 0, 19);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);

    assert_eq!(res["uri"], service_uri);
    assert_range(&res["range"], 0, 11, 0, 23);
}

#[test]
fn test_l1_type_definition_qualified_path_exact_match() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("driver_pkg2", "0.1.0").unwrap();

    let (_, ids) = ws.add_source_file_with_prefix_defs("driver_pkg2", "src/service.rs", "pub struct «DriverHandle»;\n", "driver_pkg2::service", DefKind::Struct).unwrap();
    assert_eq!(ids.len(), 1);

    // Query file registers fully-qualified path "driver_pkg2::service::DriverHandle"
    let code = "pub fn get() { let h = 1; }\n";
    ws.add_source_file("driver_pkg2", "src/lib.rs", code).unwrap();
    ws.add_type_at("driver_pkg2", "src/lib.rs", 1, 20, 1, 21, "driver_pkg2::service::DriverHandle").unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let lib_uri = ws.file_uri("driver_pkg2", "src/lib.rs").unwrap();
    let service_uri = ws.file_uri("driver_pkg2", "src/service.rs").unwrap();

    let req = make_pos_req(&lib_uri, 0, 19);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);

    assert_eq!(res["uri"], service_uri);
    assert_range(&res["range"], 0, 11, 0, 23);
}

#[test]
fn test_l1_type_definition_utf16_query_side() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_utf16_query", "0.1.0").unwrap();

    let (_, ids) = ws.add_source_file_with_defs("pkg_utf16_query", "src/models.rs", "pub struct «SuperState»;\n", DefKind::Struct).unwrap();
    assert_eq!(ids.len(), 1);

    // Query line in src/lib.rs:
    // "/* 🚀🚀 */ let s = 42;\n"
    // "/* " (3 chars, 3 UTF-16 units)
    // "🚀🚀" (2 Unicode chars, 4 UTF-16 code units with surrogate pairs)
    // " */ let " (8 chars, 8 UTF-16 units)
    // Position of `s`:
    // 1-based Unicode scalar char col: 3 + 2 + 8 + 1 = 14
    // 0-based UTF-16 character offset: 3 + 4 + 8 = 15
    let query_code = "/* 🚀🚀 */ let s = 42;\n";
    ws.add_source_file("pkg_utf16_query", "src/lib.rs", query_code).unwrap();
    // type_at is indexed by 1-based Unicode scalar char column: start_col 14, end_col 15
    ws.add_type_at("pkg_utf16_query", "src/lib.rs", 1, 14, 1, 15, "SuperState").unwrap();

    let shard = ws.build_crate_index("pkg_utf16_query").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let lib_uri = ws.file_uri("pkg_utf16_query", "src/lib.rs").unwrap();
    let models_uri = ws.file_uri("pkg_utf16_query", "src/models.rs").unwrap();

    // LSP client sends 0-based UTF-16 character 15 on line 0
    let req = make_pos_req(&lib_uri, 0, 15);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);

    // utf16_col_to_char_col converts character 15 to char column 14, hitting type_at
    assert_eq!(res["uri"], models_uri);
    assert_range(&res["range"], 0, 11, 0, 21);
}

#[test]
fn test_l1_type_definition_generic_wrapper_type_string_returns_null() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_generic_type", "0.1.0").unwrap();

    let (_, ids) = ws.add_source_file_with_defs("pkg_generic_type", "src/lib.rs", "pub struct «Config»;\n", DefKind::Struct).unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub struct Config;\npub fn run() { let list = 1; }\n";
    ws.add_source_file("pkg_generic_type", "src/lib.rs", code).unwrap();
    // type_at registers "Vec<Config>"
    ws.add_type_at("pkg_generic_type", "src/lib.rs", 2, 20, 2, 24, "Vec<Config>").unwrap();

    let shard = ws.build_crate_index("pkg_generic_type").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_generic_type", "src/lib.rs").unwrap();

    // Neither exact symbol match nor ::suffix matches "Vec<Config>", returns null
    let req = make_pos_req(&uri, 1, 20);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);

    assert_eq!(res, Value::Null);
}
