/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/references (Part 2):
//! default includeDeclaration behavior, re-exports in the same crate,
//! cross-crate multi-shard references, trait methods, and UTF-16 surrogate pairs.

use ridx::lsp::navigation::references::handle_references;
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

fn make_ref_req(uri: &str, line: u32, col: u32, include_decl: bool) -> Value {
    json!({
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": col },
            "context": { "includeDeclaration": include_decl }
        }
    })
}

fn assert_range(loc: &Value, line: u32, start_col: u32, end_col: u32) {
    assert_eq!(loc["range"]["start"]["line"], line);
    assert_eq!(loc["range"]["start"]["character"], start_col);
    assert_eq!(loc["range"]["end"]["line"], line);
    assert_eq!(loc["range"]["end"]["character"], end_col);
}

#[test]
fn test_l1_references_missing_context_or_key_defaults_to_include_decl() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_def_ctx", "0.1.0").unwrap();

    let (_, ids) = ws
        .add_source_file_with_defs(
            "pkg_def_ctx",
            "src/lib.rs",
            "pub struct «DefaultContextTarget»;\n",
            DefKind::Struct,
        )
        .unwrap();

    ws.add_refs_to_def(
        "pkg_def_ctx",
        "src/lib.rs",
        "pub struct DefaultContextTarget;\npub fn run() { let _ = «DefaultContextTarget»; }\n",
        ids[0],
    )
    .unwrap();

    let shard = ws.build_crate_index("pkg_def_ctx").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_def_ctx", "src/lib.rs").unwrap();

    // 1. Query omitting context entirely
    let res_no_ctx = handle_references(
        &make_pos_req(&uri, 0, 11),
        Some(&index),
        &overlay_mgr,
        &mut conv,
    );
    let items_no_ctx = res_no_ctx.as_array().expect("references list");

    // 2. Query with explicit includeDeclaration: true
    let res_true = handle_references(
        &make_ref_req(&uri, 0, 11, true),
        Some(&index),
        &overlay_mgr,
        &mut conv,
    );
    let items_true = res_true.as_array().expect("references list");

    // 3. Query with explicit includeDeclaration: false
    let res_false = handle_references(
        &make_ref_req(&uri, 0, 11, false),
        Some(&index),
        &overlay_mgr,
        &mut conv,
    );
    let items_false = res_false.as_array().expect("references list");

    // 4. Query with context: {} without includeDeclaration key
    let req_empty_ctx = json!({
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": 0, "character": 11 },
            "context": {}
        }
    });
    let res_empty = handle_references(&req_empty_ctx, Some(&index), &overlay_mgr, &mut conv);
    let items_empty = res_empty.as_array().expect("references list");

    // Verify unwrap_or(true): missing context matches explicit true, exactly one more than false
    assert_eq!(items_no_ctx.len(), 2);
    assert_eq!(items_true.len(), 2);
    assert_eq!(items_false.len(), 1);
    assert_eq!(items_no_ctx.len(), items_true.len());
    assert_eq!(items_no_ctx.len(), items_false.len() + 1);
    assert_eq!(items_empty.len(), items_true.len());

    assert_range(&items_no_ctx[0], 0, 11, 31);
    assert_range(&items_no_ctx[1], 1, 23, 43);
    assert_range(&items_empty[0], 0, 11, 31);
    assert_range(&items_empty[1], 1, 23, 43);
    assert_range(&items_false[0], 1, 23, 43);
}

#[test]
fn test_l1_references_reexport_via_use_same_crate() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_reexport", "0.1.0").unwrap();

    let (_, ids) = ws
        .add_source_file_with_defs(
            "pkg_reexport",
            "src/inner.rs",
            "pub struct «Worker»;\n",
            DefKind::Struct,
        )
        .unwrap();

    // lib.rs re-exports Worker from inner module
    ws.add_refs_to_def(
        "pkg_reexport",
        "src/lib.rs",
        "pub mod inner;\npub use inner::«Worker»;\n",
        ids[0],
    )
    .unwrap();

    // app.rs uses Worker
    ws.add_refs_to_def(
        "pkg_reexport",
        "src/app.rs",
        "use crate::Worker;\npub fn start() { let _ = «Worker»; }\n",
        ids[0],
    )
    .unwrap();

    let shard = ws.build_crate_index("pkg_reexport").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let inner_uri = ws.file_uri("pkg_reexport", "src/inner.rs").unwrap();
    let lib_uri = ws.file_uri("pkg_reexport", "src/lib.rs").unwrap();
    let app_uri = ws.file_uri("pkg_reexport", "src/app.rs").unwrap();

    // 1. Without declaration: exactly 2 references (the re-export in lib.rs and usage in app.rs)
    let req_no_decl = make_ref_req(&inner_uri, 0, 11, false);
    let res_no_decl = handle_references(&req_no_decl, Some(&index), &overlay_mgr, &mut conv);
    let items_no_decl = res_no_decl.as_array().expect("references list");
    assert_eq!(items_no_decl.len(), 2);
    assert_eq!(items_no_decl[0]["uri"], lib_uri);
    assert_range(&items_no_decl[0], 1, 15, 21);
    assert_eq!(items_no_decl[1]["uri"], app_uri);
    assert_range(&items_no_decl[1], 1, 25, 31);

    // 2. With declaration: exactly 3 references (declaration in inner.rs + 2 usages)
    let req_with_decl = make_ref_req(&inner_uri, 0, 11, true);
    let res_with_decl = handle_references(&req_with_decl, Some(&index), &overlay_mgr, &mut conv);
    let items_with_decl = res_with_decl.as_array().expect("references list");
    assert_eq!(items_with_decl.len(), 3);
    assert_eq!(items_with_decl[0]["uri"], inner_uri);
    assert_range(&items_with_decl[0], 0, 11, 17);
    assert_eq!(items_with_decl[1]["uri"], lib_uri);
    assert_range(&items_with_decl[1], 1, 15, 21);
    assert_eq!(items_with_decl[2]["uri"], app_uri);
    assert_range(&items_with_decl[2], 1, 25, 31);
}

#[test]
fn test_l1_references_cross_crate_two_shards() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("crate_core", "0.1.0").unwrap();
    ws.add_crate("crate_consumer", "0.1.0").unwrap();

    let (_, ids_core) = ws
        .add_source_file_with_defs(
            "crate_core",
            "src/lib.rs",
            "pub struct «ServiceConfig»;\n",
            DefKind::Struct,
        )
        .unwrap();

    // Crate consumer implements or references ServiceConfig across crate boundaries
    ws.add_source_file_with_impl_refs(
        "crate_consumer",
        "src/lib.rs",
        "use crate_core::ServiceConfig;\nimpl «ServiceConfig» {}\n",
        "crate_core",
        ids_core[0],
    )
    .unwrap();

    let _ = ws.build_crate_index("crate_core").unwrap();
    let _ = ws.build_crate_index("crate_consumer").unwrap();

    // Open workspace containing both crate shards
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    assert_eq!(index.readers().len(), 2);
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let core_uri = ws.file_uri("crate_core", "src/lib.rs").unwrap();
    let consumer_uri = ws.file_uri("crate_consumer", "src/lib.rs").unwrap();

    let req = make_ref_req(&core_uri, 0, 11, true);
    let res = handle_references(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().expect("references list");

    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["uri"], core_uri);
    assert_range(&items[0], 0, 11, 24);

    assert_eq!(items[1]["uri"], consumer_uri);
    assert_range(&items[1], 1, 5, 18);
}

#[test]
fn test_l1_references_trait_method_indexed_refs() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_trait_method", "0.1.0").unwrap();

    let (_, ids) = ws
        .add_source_file_with_defs(
            "pkg_trait_method",
            "src/lib.rs",
            "pub trait Storage {\n    fn «save»(&self) -> bool;\n}\n",
            DefKind::AssocFn,
        )
        .unwrap();

    ws.add_refs_to_def(
        "pkg_trait_method",
        "src/client.rs",
        "use crate::Storage;\npub fn do_save<T: Storage>(s: &T) { s.«save»(); }\n",
        ids[0],
    )
    .unwrap();

    let shard = ws.build_crate_index("pkg_trait_method").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let lib_uri = ws.file_uri("pkg_trait_method", "src/lib.rs").unwrap();
    let client_uri = ws.file_uri("pkg_trait_method", "src/client.rs").unwrap();

    // reference is indexed synthetically via add_refs_to_def; pins handler output for an AssocFn target, not method resolution
    let req = make_ref_req(&lib_uri, 1, 7, false);
    let res = handle_references(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().expect("references list");

    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["uri"], client_uri);
    assert_range(&items[0], 1, 38, 42);
}

#[test]
fn test_l1_references_utf16_multibyte_prefix_columns() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_utf16", "0.1.0").unwrap();

    let (_, ids) = ws
        .add_source_file_with_defs(
            "pkg_utf16",
            "src/lib.rs",
            "pub struct «Rocket»;\n",
            DefKind::Struct,
        )
        .unwrap();

    // 4-byte UTF-8 emoji (\u{1F680}) has 2 UTF-16 code units; prefix length is 35 UTF-16 units.
    ws.add_refs_to_def(
        "pkg_utf16",
        "src/lib.rs",
        "pub struct Rocket;\n/* \u{1F680} */ pub fn launch() { let _ = «Rocket»; }\n",
        ids[0],
    )
    .unwrap();

    let shard = ws.build_crate_index("pkg_utf16").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_utf16", "src/lib.rs").unwrap();

    let req = make_ref_req(&uri, 0, 11, false);
    let res = handle_references(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().expect("references list");
    assert_eq!(items.len(), 1);
    assert_range(&items[0], 1, 35, 41);
}
