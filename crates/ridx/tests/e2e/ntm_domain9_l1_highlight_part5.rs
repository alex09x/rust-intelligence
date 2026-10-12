/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/documentHighlight part 5:
//! cross-file boundary isolation, query from reference sites, and zero/multiple references.

use ridx::lsp::navigation::handle_document_highlight;
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

#[test]
fn test_document_highlight_cross_file_def_only_returns_local_refs() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_cross", "0.1.0").unwrap();

    let (_, other_ids) = ws
        .add_source_file_with_defs(
            "pkg_cross",
            "src/other.rs",
            "pub struct «ForeignModel»;\n",
            DefKind::Struct,
        )
        .unwrap();

    let lib_refs = "pub fn execute() {\n    let _ = «ForeignModel»;\n    let _ = «ForeignModel»;\n}\n";
    ws.add_refs_to_def("pkg_cross", "src/lib.rs", lib_refs, other_ids[0])
        .unwrap();

    let shard = ws.build_crate_index("pkg_cross").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let uri = ws.file_uri("pkg_cross", "src/lib.rs").unwrap();

    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let req = make_pos_req(&uri, 1, 14);
    let res = handle_document_highlight(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["kind"], 1);
    assert_eq!(items[0]["range"]["start"]["line"], 1);
    assert_eq!(items[1]["kind"], 1);
    assert_eq!(items[1]["range"]["start"]["line"], 2);
}

#[test]
fn test_document_highlight_cross_file_ref_omitted_from_target_doc() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_isolated", "0.1.0").unwrap();

    let (_, ids) = ws
        .add_source_file_with_defs(
            "pkg_isolated",
            "src/lib.rs",
            "pub struct «LocalDef»;\n",
            DefKind::Struct,
        )
        .unwrap();

    ws.add_refs_to_def(
        "pkg_isolated",
        "src/lib.rs",
        "pub struct LocalDef;\npub fn local_user() {\n    let _ = «LocalDef»;\n}\n",
        ids[0],
    )
    .unwrap();

    ws.add_refs_to_def(
        "pkg_isolated",
        "src/other.rs",
        "pub fn foreign_user() {\n    let _ = «LocalDef»;\n}\n",
        ids[0],
    )
    .unwrap();

    let shard = ws.build_crate_index("pkg_isolated").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let uri = ws.file_uri("pkg_isolated", "src/lib.rs").unwrap();

    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let req = make_pos_req(&uri, 0, 12);
    let res = handle_document_highlight(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["kind"], 3);
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[1]["kind"], 1);
    assert_eq!(items[1]["range"]["start"]["line"], 2);
}

#[test]
fn test_document_highlight_compiled_query_from_reference_location() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_refquery", "0.1.0").unwrap();

    let (_, ids) = ws
        .add_source_file_with_defs(
            "pkg_refquery",
            "src/lib.rs",
            "pub struct «Anchor»;\n",
            DefKind::Struct,
        )
        .unwrap();

    let refs_code = "pub struct Anchor;\npub fn use_anchor() {\n    let _ = «Anchor»;\n}\n";
    ws.add_refs_to_def("pkg_refquery", "src/lib.rs", refs_code, ids[0])
        .unwrap();

    let shard = ws.build_crate_index("pkg_refquery").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let uri = ws.file_uri("pkg_refquery", "src/lib.rs").unwrap();

    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let req = make_pos_req(&uri, 2, 14);
    let res = handle_document_highlight(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["kind"], 3);
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[1]["kind"], 1);
    assert_eq!(items[1]["range"]["start"]["line"], 2);
}

#[test]
fn test_document_highlight_compiled_zero_refs_returns_only_declaration() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_solitary", "0.1.0").unwrap();

    ws.add_source_file_with_defs(
        "pkg_solitary",
        "src/lib.rs",
        "pub struct «Solitary»;\n",
        DefKind::Struct,
    )
    .unwrap();

    let shard = ws.build_crate_index("pkg_solitary").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let uri = ws.file_uri("pkg_solitary", "src/lib.rs").unwrap();

    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let req = make_pos_req(&uri, 0, 12);
    let res = handle_document_highlight(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["kind"], 3);
    assert_eq!(items[0]["range"]["start"]["line"], 0);
}

#[test]
fn test_document_highlight_compiled_non_existent_symbol_returns_empty() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_empty", "0.1.0").unwrap();

    ws.add_source_file(
        "pkg_empty",
        "src/lib.rs",
        "// Just a blank file without indexed symbols\n\nfn unused() {}\n",
    )
    .unwrap();

    let shard = ws.build_crate_index("pkg_empty").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let uri = ws.file_uri("pkg_empty", "src/lib.rs").unwrap();

    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let req = make_pos_req(&uri, 0, 5);
    let res = handle_document_highlight(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]));
}

#[test]
fn test_document_highlight_compiled_type_alias_and_references() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_alias", "0.1.0").unwrap();

    let (_, ids) = ws
        .add_source_file_with_defs(
            "pkg_alias",
            "src/lib.rs",
            "pub type «Handle» = u64;\n",
            DefKind::TyAlias,
        )
        .unwrap();

    let refs_code = "pub type Handle = u64;\npub fn open_handle() -> «Handle» {\n    let h: «Handle» = 100;\n    h\n}\n";
    ws.add_refs_to_def("pkg_alias", "src/lib.rs", refs_code, ids[0])
        .unwrap();

    let shard = ws.build_crate_index("pkg_alias").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let uri = ws.file_uri("pkg_alias", "src/lib.rs").unwrap();

    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let req = make_pos_req(&uri, 0, 10);
    let res = handle_document_highlight(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["kind"], 3);
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[1]["kind"], 1);
    assert_eq!(items[1]["range"]["start"]["line"], 1);
    assert_eq!(items[2]["kind"], 1);
    assert_eq!(items[2]["range"]["start"]["line"], 2);
}

#[test]
fn test_document_highlight_compiled_multiple_references_same_line() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_sameline", "0.1.0").unwrap();

    let (_, ids) = ws
        .add_source_file_with_defs(
            "pkg_sameline",
            "src/lib.rs",
            "pub struct «Pair»;\n",
            DefKind::Struct,
        )
        .unwrap();

    let refs_code = "pub struct Pair;\npub fn make_pair() {\n    let _ = («Pair», «Pair»);\n}\n";
    ws.add_refs_to_def("pkg_sameline", "src/lib.rs", refs_code, ids[0])
        .unwrap();

    let shard = ws.build_crate_index("pkg_sameline").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let uri = ws.file_uri("pkg_sameline", "src/lib.rs").unwrap();

    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let req = make_pos_req(&uri, 0, 12);
    let res = handle_document_highlight(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["kind"], 3);
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[1]["kind"], 1);
    assert_eq!(items[1]["range"]["start"]["line"], 2);
    assert_eq!(items[2]["kind"], 1);
    assert_eq!(items[2]["range"]["start"]["line"], 2);
}
