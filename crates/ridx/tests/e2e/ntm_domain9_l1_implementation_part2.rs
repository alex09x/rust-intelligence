/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Hermetic integration tests for Domain 9: textDocument/implementation (Part 2).

use ridx::lsp::navigation::handle_implementation;
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
fn test_l1_implementation_struct_inherent_and_two_traits() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_three_impls", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs(
            "pkg_three_impls",
            "src/lib.rs",
            "pub struct «Engine»;\n",
            DefKind::Struct,
        )
        .unwrap();
    assert_eq!(ids.len(), 1);

    let code = "\
pub struct Engine;
«impl Engine» {}
«impl Starter for Engine» {}
«impl Stopper for Engine» {}
";
    ws.add_source_file_with_impl_refs(
        "pkg_three_impls",
        "src/lib.rs",
        code,
        "pkg_three_impls",
        ids[0],
    )
    .unwrap();

    let shard = ws.build_crate_index("pkg_three_impls").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_three_impls", "src/lib.rs").unwrap();

    // Query on `Engine` definition (line 0, col 11)
    let req = make_pos_req(&uri, 0, 11);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().expect("impl list");

    assert_eq!(items.len(), 3, "Must find inherent and both trait impls");
    assert_eq!(items[0]["uri"], uri);
    assert_eq!(items[0]["range"]["start"]["line"], 1);
    assert_eq!(items[0]["range"]["start"]["character"], 0);
    assert_eq!(items[0]["range"]["end"]["character"], 11);

    assert_eq!(items[1]["uri"], uri);
    assert_eq!(items[1]["range"]["start"]["line"], 2);
    assert_eq!(items[1]["range"]["start"]["character"], 0);
    assert_eq!(items[1]["range"]["end"]["character"], 23);

    assert_eq!(items[2]["uri"], uri);
    assert_eq!(items[2]["range"]["start"]["line"], 3);
    assert_eq!(items[2]["range"]["start"]["character"], 0);
    assert_eq!(items[2]["range"]["end"]["character"], 23);
}

#[test]
fn test_l1_implementation_trait_without_implementations() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_unimplemented", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs(
            "pkg_unimplemented",
            "src/lib.rs",
            "pub trait «LonelyTrait» {}\n",
            DefKind::Trait,
        )
        .unwrap();
    assert_eq!(ids.len(), 1);

    let shard = ws.build_crate_index("pkg_unimplemented").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_unimplemented", "src/lib.rs").unwrap();

    let req = make_pos_req(&uri, 0, 12);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);

    assert_eq!(res, json!([]), "Unimplemented trait must return empty array");
}

#[test]
fn test_l1_implementation_two_separate_files_same_crate() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_two_files", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs(
            "pkg_two_files",
            "src/logger.rs",
            "pub trait «Logger» {}\n",
            DefKind::Trait,
        )
        .unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file_with_impl_refs(
        "pkg_two_files",
        "src/console.rs",
        "«impl Logger for ConsoleLogger» {}\n",
        "pkg_two_files",
        ids[0],
    )
    .unwrap();

    ws.add_source_file_with_impl_refs(
        "pkg_two_files",
        "src/file.rs",
        "«impl Logger for FileLogger» {}\n",
        "pkg_two_files",
        ids[0],
    )
    .unwrap();

    let shard = ws.build_crate_index("pkg_two_files").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let logger_uri = ws.file_uri("pkg_two_files", "src/logger.rs").unwrap();
    let console_uri = ws.file_uri("pkg_two_files", "src/console.rs").unwrap();
    let file_uri = ws.file_uri("pkg_two_files", "src/file.rs").unwrap();

    let req = make_pos_req(&logger_uri, 0, 11);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().expect("impl list");

    assert_eq!(items.len(), 2, "Must find implementations in both files");
    assert_eq!(items[0]["uri"], console_uri);
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[0]["range"]["start"]["character"], 0);
    assert_eq!(items[0]["range"]["end"]["character"], 29);

    assert_eq!(items[1]["uri"], file_uri);
    assert_eq!(items[1]["range"]["start"]["line"], 0);
    assert_eq!(items[1]["range"]["start"]["character"], 0);
    assert_eq!(items[1]["range"]["end"]["character"], 26);
}

#[test]
fn test_l1_implementation_cursor_on_impl_header_returns_self_impl() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_impl_header", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs(
            "pkg_impl_header",
            "src/lib.rs",
            "pub trait «Worker» {}\npub struct Task;\n",
            DefKind::Trait,
        )
        .unwrap();

    let code = "pub trait Worker {}\npub struct Task;\n«impl Worker for Task» {}\n";
    ws.add_source_file_with_impl_refs(
        "pkg_impl_header",
        "src/lib.rs",
        code,
        "pkg_impl_header",
        ids[0],
    )
    .unwrap();

    let shard = ws.build_crate_index("pkg_impl_header").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_impl_header", "src/lib.rs").unwrap();

    // Cursor on the `impl` keyword itself on line 2, col 2 (impl definition itself)
    let req = make_pos_req(&uri, 2, 2);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().expect("impl list");

    // Current RIDX behavior, pinned: returns the impl block definition itself
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["uri"], uri);
    assert_eq!(items[0]["range"]["start"]["line"], 2);
    assert_eq!(items[0]["range"]["start"]["character"], 0);
    assert_eq!(items[0]["range"]["end"]["character"], 20);
}

#[test]
fn test_l1_implementation_generic_impl_block() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_generic_impl", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs(
            "pkg_generic_impl",
            "src/lib.rs",
            "pub trait «Serializable» {}\n",
            DefKind::Trait,
        )
        .unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub trait Serializable {}\npub struct Container<T>(T);\n«impl<T> Serializable for Container<T>» {}\n";
    ws.add_source_file_with_impl_refs(
        "pkg_generic_impl",
        "src/lib.rs",
        code,
        "pkg_generic_impl",
        ids[0],
    )
    .unwrap();

    let shard = ws.build_crate_index("pkg_generic_impl").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_generic_impl", "src/lib.rs").unwrap();

    let req = make_pos_req(&uri, 0, 12);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().expect("impl list");

    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["uri"], uri);
    assert_eq!(items[0]["range"]["start"]["line"], 2);
    assert_eq!(items[0]["range"]["start"]["character"], 0);
    assert_eq!(items[0]["range"]["end"]["character"], 37);
}

#[test]
fn test_l1_implementation_utf16_with_emoji_prefix() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_impl_utf16", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs(
            "pkg_impl_utf16",
            "src/lib.rs",
            "pub trait «Action» {}\n",
            DefKind::Trait,
        )
        .unwrap();
    assert_eq!(ids.len(), 1);

    // 2 crabs: 4 UTF-16 code units. "/* 🦀🦀 */ " = 3 + 4 + 4 = 11 UTF-16 code units
    let code = "pub trait Action {}\n/* 🦀🦀 */ «impl Action for ()» {}\n";
    ws.add_source_file_with_impl_refs(
        "pkg_impl_utf16",
        "src/lib.rs",
        code,
        "pkg_impl_utf16",
        ids[0],
    )
    .unwrap();

    let shard = ws.build_crate_index("pkg_impl_utf16").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_impl_utf16", "src/lib.rs").unwrap();

    let req = make_pos_req(&uri, 0, 11);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().expect("impl list");

    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["uri"], uri);
    assert_eq!(items[0]["range"]["start"]["line"], 1);
    assert_eq!(items[0]["range"]["start"]["character"], 11);
    assert_eq!(items[0]["range"]["end"]["character"], 29);
}
