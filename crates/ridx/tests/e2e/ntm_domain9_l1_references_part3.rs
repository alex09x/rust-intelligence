/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/references (Part 3):
//! multiple references on the same line and live dirty overlay mapped references.

use ridx::lsp::navigation::references::handle_references;
use ridx::lsp::protocol::PositionConverter;
use ridx::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use ridx_test_support::fixture::SyntheticWorkspaceFixture;
use serde_json::{json, Value};

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
fn test_l1_references_multiple_references_same_line() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_multi_ref", "0.1.0").unwrap();

    let (_, ids) = ws
        .add_source_file_with_defs(
            "pkg_multi_ref",
            "src/lib.rs",
            "pub fn «calc»(v: i32) -> i32 { v }\n",
            DefKind::Fn,
        )
        .unwrap();

    ws.add_refs_to_def(
        "pkg_multi_ref",
        "src/lib.rs",
        "pub fn calc(v: i32) -> i32 { v }\npub fn nested() { let _ = «calc»(«calc»(5)); }\n",
        ids[0],
    )
    .unwrap();

    let shard = ws.build_crate_index("pkg_multi_ref").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_multi_ref", "src/lib.rs").unwrap();

    let req = make_ref_req(&uri, 0, 7, false);
    let res = handle_references(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().expect("references list");

    // Exactly 2 usages on line 1 with different columns
    assert_eq!(items.len(), 2);
    assert_range(&items[0], 1, 26, 30);
    assert_range(&items[1], 1, 31, 35);
}

#[test]
fn test_l1_references_overlay_dirty_mapped_references() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_dirty", "0.1.0").unwrap();

    let (_, ids) = ws
        .add_source_file_with_defs(
            "pkg_dirty",
            "src/lib.rs",
            "pub struct «DirtyItem»;\n",
            DefKind::Struct,
        )
        .unwrap();

    ws.add_refs_to_def(
        "pkg_dirty",
        "src/lib.rs",
        "pub struct DirtyItem;\npub fn run() { let _ = «DirtyItem»; }\n",
        ids[0],
    )
    .unwrap();

    let shard = ws.build_crate_index("pkg_dirty").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_dirty", "src/lib.rs").unwrap();
    let file = ws.file_path("pkg_dirty", "src/lib.rs").unwrap();
    let file_str = file.to_str().unwrap();

    let compiled_code = "pub struct DirtyItem;\npub fn run() { let _ = DirtyItem; }\n";
    overlay_mgr.open_document(&uri, file_str, 1, compiled_code.to_string());

    // Insert 2 lines at top in overlay via update
    let live_code = "// header\n\npub struct DirtyItem;\npub fn run() { let _ = DirtyItem; }\n";
    overlay_mgr.update(file_str, 2, live_code.to_string());

    // Query on definition at shifted live line 2
    let req = make_ref_req(&uri, 2, 11, true);
    let res = handle_references(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().expect("references list");

    // Both definition (line 2, col 11..20) and usage (line 3, col 23..32) are mapped to live shifted lines
    assert_eq!(items.len(), 2);
    assert_range(&items[0], 2, 11, 20);
    assert_range(&items[1], 3, 23, 32);
}
