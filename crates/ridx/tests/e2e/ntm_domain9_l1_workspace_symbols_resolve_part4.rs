/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspaceSymbol/resolve part 4:
//! multi-crate ordering, deep nesting containers, UTF-16 surrogate handling, and macros.

use ridx::lsp::protocol::PositionConverter;
use ridx::lsp::symbols::handle_workspace_symbol_resolve;
use ridx_format::{DefKind, WorkspaceIndex};
use ridx_test_support::fixture::SyntheticWorkspaceFixture;
use serde_json::json;

#[test]
fn test_workspace_symbol_resolve_multi_crate_first_shard_wins() {
    let mut ws1 = SyntheticWorkspaceFixture::new().unwrap();
    ws1.add_crate("crate1", "0.1.0").unwrap();
    ws1.add_source_file_with_defs("crate1", "src/lib.rs", "pub struct «DuplicateName»;\n", DefKind::Struct).unwrap();
    ws1.build_all_indices().unwrap();
    let idx1 = WorkspaceIndex::open(ws1.root_path()).unwrap();

    let mut ws2 = SyntheticWorkspaceFixture::new().unwrap();
    ws2.add_crate("crate2", "0.1.0").unwrap();
    ws2.add_source_file_with_defs("crate2", "src/lib.rs", "pub struct «DuplicateName»;\n", DefKind::Struct).unwrap();
    ws2.build_all_indices().unwrap();
    let idx2 = WorkspaceIndex::open(ws2.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "name": "DuplicateName" }
    });
    // First shard wins in iteration order
    let res = handle_workspace_symbol_resolve(&req, [&idx1, &idx2].into_iter(), &mut conv);
    assert_eq!(res["name"], "DuplicateName");
    assert_eq!(res["containerName"], "crate1");
}

#[test]
fn test_workspace_symbol_resolve_multi_crate_second_shard_when_first_lacks() {
    let mut ws1 = SyntheticWorkspaceFixture::new().unwrap();
    ws1.add_crate("crate1", "0.1.0").unwrap();
    ws1.add_source_file_with_defs("crate1", "src/lib.rs", "pub struct «OnlyInCrate1»;\n", DefKind::Struct).unwrap();
    ws1.build_all_indices().unwrap();
    let idx1 = WorkspaceIndex::open(ws1.root_path()).unwrap();

    let mut ws2 = SyntheticWorkspaceFixture::new().unwrap();
    ws2.add_crate("crate2", "0.1.0").unwrap();
    ws2.add_source_file_with_defs("crate2", "src/lib.rs", "pub struct «OnlyInCrate2»;\n", DefKind::Struct).unwrap();
    ws2.build_all_indices().unwrap();
    let idx2 = WorkspaceIndex::open(ws2.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "name": "OnlyInCrate2" }
    });
    let res = handle_workspace_symbol_resolve(&req, [&idx1, &idx2].into_iter(), &mut conv);
    assert_eq!(res["name"], "OnlyInCrate2");
    assert_eq!(res["containerName"], "crate2");
}

#[test]
fn test_workspace_symbol_resolve_container_name_deep_nested_module() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_deep_res", "0.1.0").unwrap();
    ws.add_source_file_with_prefix_defs("pkg_deep_res", "src/a/b/c.rs", "pub struct «DeepNestRecord»;\n", "pkg_deep_res::a::b::c", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "name": "DeepNestRecord" }
    });
    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&idx), &mut conv);
    assert_eq!(res["name"], "DeepNestRecord");
    assert_eq!(res["containerName"], "c");
}

#[test]
fn test_workspace_symbol_resolve_utf16_surrogate_emoji_offset() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_utf16_res", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_utf16_res", "src/lib.rs", "// Emoji line: 🦀🦀\npub struct «EmojiTarget»;\n", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "name": "EmojiTarget" }
    });
    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&idx), &mut conv);
    assert_eq!(res["name"], "EmojiTarget");
    assert_eq!(res["location"]["range"]["start"]["line"], 1);
    assert_eq!(res["location"]["range"]["start"]["character"], 11);
    assert_eq!(res["location"]["range"]["end"]["character"], 22);
}

#[test]
fn test_workspace_symbol_resolve_crlf_offsets_preserved() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_crlf_res", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_crlf_res", "src/lib.rs", "pub struct «CrlfTarget»;\r\n", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "name": "CrlfTarget" }
    });
    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&idx), &mut conv);
    assert_eq!(res["name"], "CrlfTarget");
    assert_eq!(res["location"]["range"]["start"]["line"], 0);
}

#[test]
fn test_workspace_symbol_resolve_macro_def_kind() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_macro_res", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_macro_res", "src/lib.rs", "macro_rules! «my_macro» { () => {} }\n", DefKind::Macro).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "name": "my_macro" }
    });
    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&idx), &mut conv);
    assert_eq!(res["name"], "my_macro");
    assert_eq!(res["containerName"], "pkg_macro_res");
    assert_eq!(res["location"]["range"]["start"]["line"], 0);
}

#[test]
fn test_workspace_symbol_resolve_union_def_kind() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_union_res", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_union_res", "src/lib.rs", "pub union «ResolvedUnion» { f: f32, u: u32 }\n", DefKind::Union).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "name": "ResolvedUnion" }
    });
    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&idx), &mut conv);
    assert_eq!(res["name"], "ResolvedUnion");
    assert_eq!(res["containerName"], "pkg_union_res");
    assert_eq!(res["location"]["range"]["start"]["line"], 0);
}

#[test]
fn test_workspace_symbol_resolve_id_preservation() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_id_pres", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_id_pres", "src/lib.rs", "pub struct «IdPreserveTarget»;\n", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": {
            "name": "IdPreserveTarget",
            "id": "req-98765"
        }
    });
    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&idx), &mut conv);
    assert_eq!(res["name"], "IdPreserveTarget");
    assert_eq!(res["id"], "req-98765");
    assert_eq!(res["location"]["range"]["start"]["line"], 0);
}

#[test]
fn test_workspace_symbol_resolve_client_version_field_preservation() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_ver_pres", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_ver_pres", "src/lib.rs", "pub struct «VersionedSymbol»;\n", DefKind::Struct).unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": {
            "name": "VersionedSymbol",
            "version": 42
        }
    });
    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&idx), &mut conv);
    assert_eq!(res["name"], "VersionedSymbol");
    assert_eq!(res["version"], 42);
    assert_eq!(res["location"]["range"]["start"]["line"], 0);
}
