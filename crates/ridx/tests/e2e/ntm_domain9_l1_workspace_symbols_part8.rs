/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspace/symbol part 8:
//! single-letter queries, null query fallback, module containers, and UTF-16 columns.

use ridx::lsp::protocol::PositionConverter;
use ridx::lsp::symbols::handle_workspace_symbol;
use ridx_format::{DefKind, WorkspaceIndex};
use ridx_test_support::fixture::SyntheticWorkspaceFixture;
use serde_json::json;

#[test]
fn test_workspace_symbol_single_letter_query() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_single_let", "0.1.0").unwrap();
    ws.add_source_file_with_defs(
        "pkg_single_let",
        "src/lib.rs",
        "pub struct «XRay»;\npub struct «Alpha»;\n",
        DefKind::Struct,
    )
    .unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "query": "X" }
    });
    let res = handle_workspace_symbol(&req, [&idx].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "XRay");
}

#[test]
fn test_workspace_symbol_null_query_treated_as_empty() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_null_q", "0.1.0").unwrap();
    ws.add_source_file_with_defs(
        "pkg_null_q",
        "src/lib.rs",
        "pub struct «NullTarget»;\n",
        DefKind::Struct,
    )
    .unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "query": serde_json::Value::Null }
    });
    let res = handle_workspace_symbol(&req, [&idx].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "NullTarget");
}

#[test]
fn test_workspace_symbol_special_chars_in_filename_uri() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_special_fn", "0.1.0").unwrap();
    ws.add_source_file_with_defs(
        "pkg_special_fn",
        "src/my_util.rs",
        "pub struct «SpecialUtil»;\n",
        DefKind::Struct,
    )
    .unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "query": "SpecialUtil" }
    });
    let res = handle_workspace_symbol(&req, [&idx].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 1);
    let uri = items[0]["location"]["uri"].as_str().unwrap();
    assert!(uri.ends_with("my_util.rs"));
}

#[test]
fn test_workspace_symbol_many_symbols_below_limit() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_many", "0.1.0").unwrap();
    ws.add_source_file_with_defs(
        "pkg_many",
        "src/lib.rs",
        "pub struct «Item01»;\npub struct «Item02»;\npub struct «Item03»;\npub struct «Item04»;\npub struct «Item05»;\n",
        DefKind::Struct,
    )
    .unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "query": "Item0" }
    });
    let res = handle_workspace_symbol(&req, [&idx].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 5);
}

#[test]
fn test_workspace_symbol_utf16_multibyte_name_location() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_multibyte", "0.1.0").unwrap();
    ws.add_source_file_with_defs(
        "pkg_multibyte",
        "src/lib.rs",
        "// UTF8 prefix 🦀\npub struct «UnicodeAnchor»;\n",
        DefKind::Struct,
    )
    .unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "query": "UnicodeAnchor" }
    });
    let res = handle_workspace_symbol(&req, [&idx].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "UnicodeAnchor");
    assert_eq!(items[0]["location"]["range"]["start"]["line"], 1);
}

#[test]
fn test_workspace_symbol_nested_modules_with_struct_and_enum() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_nested_kinds", "0.1.0").unwrap();
    ws.add_source_file_with_prefix_defs(
        "pkg_nested_kinds",
        "src/core/types.rs",
        "pub struct «CoreStruct»;\npub enum «CoreEnum» { Val }\n",
        "pkg_nested_kinds::core::types",
        DefKind::Struct,
    )
    .unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "query": "Core" }
    });
    let res = handle_workspace_symbol(&req, [&idx].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["containerName"], "types");
    assert_eq!(items[1]["containerName"], "types");
}

#[test]
fn test_workspace_symbol_query_matching_case_variations() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_case_var", "0.1.0").unwrap();
    ws.add_source_file_with_defs(
        "pkg_case_var",
        "src/lib.rs",
        "pub struct «TargetVariant»;\n",
        DefKind::Struct,
    )
    .unwrap();
    ws.build_all_indices().unwrap();
    let idx = WorkspaceIndex::open(ws.root_path()).unwrap();

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": { "query": "tArGeTvArIaNt" }
    });
    let res = handle_workspace_symbol(&req, [&idx].into_iter(), &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "TargetVariant");
}
