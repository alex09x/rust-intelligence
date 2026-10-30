/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Hermetic integration tests for Domain 9: textDocument/typeDefinition and textDocument/implementation.

use ridx::lsp::navigation::{handle_implementation, handle_type_definition};
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
fn test_l1_type_definition_single_file() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_a", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_a", "src/lib.rs", "pub struct «Bar»;\n", DefKind::Struct).unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file("pkg_a", "src/lib.rs", "pub struct Bar;\npub fn foo() { let val = 42; }\n").unwrap();
    // Inferred type of `val` at line 2 cols 20..23 is "Bar"
    ws.add_type_at("pkg_a", "src/lib.rs", 2, 20, 2, 23, "Bar").unwrap();

    let shard = ws.build_crate_index("pkg_a").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_a", "src/lib.rs").unwrap();

    // Cursor on `val` on line 1 (0-based), col 20
    let req = make_pos_req(&uri, 1, 20);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert!(!res.is_null(), "Type definition must resolve to struct Bar");
    assert_eq!(res["uri"], uri);
    assert_eq!(res["range"]["start"]["line"], 0);
    assert_eq!(res["range"]["start"]["character"], 11);
    assert_eq!(res["range"]["end"]["character"], 14);
}

#[test]
fn test_l1_type_definition_cross_file() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_cross", "0.1.0").unwrap();

    // Type `UserToken` defined in types.rs
    ws.add_source_file_with_defs("pkg_cross", "src/types.rs", "pub struct «UserToken»;\n", DefKind::Struct).unwrap();

    // Function parameter in handler.rs with type UserToken
    ws.add_source_file("pkg_cross", "src/handler.rs", "use crate::types::UserToken;\npub fn handle(token: UserToken) {}\n").unwrap();
    ws.add_type_at("pkg_cross", "src/handler.rs", 2, 15, 2, 20, "UserToken").unwrap();

    let shard = ws.build_crate_index("pkg_cross").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let handler_uri = ws.file_uri("pkg_cross", "src/handler.rs").unwrap();
    let types_uri = ws.file_uri("pkg_cross", "src/types.rs").unwrap();

    let req = make_pos_req(&handler_uri, 1, 16);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert!(!res.is_null(), "Cross-file type definition must resolve");
    assert_eq!(res["uri"], types_uri);
    assert_eq!(res["range"]["start"]["line"], 0);
}

#[test]
fn test_l1_type_definition_enum_binding() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_enum", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_enum", "src/lib.rs", "pub enum «Status» { Active, Inactive }\n", DefKind::Enum).unwrap();
    ws.add_source_file("pkg_enum", "src/lib.rs", "pub enum Status { Active, Inactive }\npub fn check() { let s = Status::Active; }\n").unwrap();
    ws.add_type_at("pkg_enum", "src/lib.rs", 2, 22, 2, 24, "Status").unwrap();

    let shard = ws.build_crate_index("pkg_enum").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_enum", "src/lib.rs").unwrap();

    let req = make_pos_req(&uri, 1, 21);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert!(!res.is_null());
    assert_eq!(res["uri"], uri);
    assert_eq!(res["range"]["start"]["line"], 0);
}

#[test]
fn test_l1_type_definition_dirty_overlay_returns_null() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_dirty", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_dirty", "src/lib.rs", "pub struct «Target»;\n", DefKind::Struct).unwrap();
    ws.add_type_at("pkg_dirty", "src/lib.rs", 1, 12, 1, 18, "Target").unwrap();

    let shard = ws.build_crate_index("pkg_dirty").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_dirty", "src/lib.rs").unwrap();
    let file_path = ws.file_path("pkg_dirty", "src/lib.rs").unwrap();
    let file_str = file_path.to_str().unwrap();

    // Open overlay and make it dirty
    overlay_mgr.open_document(&uri, file_str, 1, "pub struct Target;\n".to_string());
    overlay_mgr.update(file_str, 2, "// dirty edit\npub struct Target;\n".to_string());

    let req = make_pos_req(&uri, 1, 12);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert!(res.is_null(), "Dirty overlay must return null for typeDefinition");
}

#[test]
fn test_l1_type_definition_whitespace_and_oob_bounds() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_bounds", "0.1.0").unwrap();
    ws.add_source_file_with_defs("pkg_bounds", "src/lib.rs", "pub struct «Entity»;\n", DefKind::Struct).unwrap();

    let shard = ws.build_crate_index("pkg_bounds").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_bounds", "src/lib.rs").unwrap();

    // Cursor on whitespace
    let ws_req = make_pos_req(&uri, 0, 0);
    let ws_res = handle_type_definition(&ws_req, Some(&index), &overlay_mgr, &mut conv);
    assert!(ws_res.is_null());

    // Cursor out of bounds
    let oob_req = make_pos_req(&uri, 999, 999);
    let oob_res = handle_type_definition(&oob_req, Some(&index), &overlay_mgr, &mut conv);
    assert!(oob_res.is_null());

    // Missing workspace
    let no_ws = handle_type_definition(&ws_req, None, &overlay_mgr, &mut conv);
    assert!(no_ws.is_null());
}

#[test]
fn test_l1_implementation_trait_single_file() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_impl", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_impl", "src/lib.rs", "pub trait «Greeter» {}\n", DefKind::Trait).unwrap();

    ws.add_source_file_with_impl_refs("pkg_impl", "src/lib.rs", "pub trait Greeter {}\npub struct Person;\n«impl Greeter for Person» {}\n", "pkg_impl", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_impl").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_impl", "src/lib.rs").unwrap();

    // Cursor on `Greeter` definition at line 0, col 11
    let req = make_pos_req(&uri, 0, 11);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().expect("implementation list");
    assert_eq!(items.len(), 1, "Must find 1 implementation");
    assert_eq!(items[0]["uri"], uri);
    assert_eq!(items[0]["range"]["start"]["line"], 2);
}

#[test]
fn test_l1_implementation_trait_cross_file() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_cross_impl", "0.1.0").unwrap();

    let (_, ids) = ws.add_source_file_with_defs("pkg_cross_impl", "src/traits.rs", "pub trait «Renderer» {}\n", DefKind::Trait).unwrap();

    ws.add_source_file_with_impl_refs(
        "pkg_cross_impl",
        "src/renderers.rs",
        "use crate::traits::Renderer;\npub struct Canvas;\n«impl Renderer for Canvas» {}\n",
        "pkg_cross_impl",
        ids[0],
    )
    .unwrap();

    let shard = ws.build_crate_index("pkg_cross_impl").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let traits_uri = ws.file_uri("pkg_cross_impl", "src/traits.rs").unwrap();
    let renderers_uri = ws.file_uri("pkg_cross_impl", "src/renderers.rs").unwrap();

    let req = make_pos_req(&traits_uri, 0, 11);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().expect("implementation list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["uri"], renderers_uri);
    assert_eq!(items[0]["range"]["start"]["line"], 2);
}

#[test]
fn test_l1_implementation_multiple_implementations() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_multi", "0.1.0").unwrap();

    let (_, ids) = ws.add_source_file_with_defs("pkg_multi", "src/traits.rs", "pub trait «Worker» {}\n", DefKind::Trait).unwrap();

    // First impl in cpu.rs
    ws.add_source_file_with_impl_refs("pkg_multi", "src/cpu.rs", "use crate::traits::Worker;\npub struct Cpu;\n«impl Worker for Cpu» {}\n", "pkg_multi", ids[0]).unwrap();

    // Second impl in gpu.rs
    ws.add_source_file_with_impl_refs("pkg_multi", "src/gpu.rs", "use crate::traits::Worker;\npub struct Gpu;\n«impl Worker for Gpu» {}\n", "pkg_multi", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_multi").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let traits_uri = ws.file_uri("pkg_multi", "src/traits.rs").unwrap();

    let req = make_pos_req(&traits_uri, 0, 11);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().expect("implementation list");
    assert_eq!(items.len(), 2, "Must find both Cpu and Gpu implementations");
}

#[test]
fn test_l1_implementation_dirty_overlay_and_boundaries() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_bounds", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_bounds", "src/lib.rs", "pub trait «Serializable» {}\n", DefKind::Trait).unwrap();
    ws.add_source_file_with_impl_refs("pkg_bounds", "src/lib.rs", "pub trait Serializable {}\npub struct Data;\n«impl Serializable for Data» {}\n", "pkg_bounds", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_bounds").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_bounds", "src/lib.rs").unwrap();
    let file_path = ws.file_path("pkg_bounds", "src/lib.rs").unwrap();
    let file_str = file_path.to_str().unwrap();

    // Dirty overlay returns empty array
    overlay_mgr.open_document(&uri, file_str, 1, "pub trait Serializable {}\n".to_string());
    overlay_mgr.update(file_str, 2, "// dirty\npub trait Serializable {}\n".to_string());

    let req = make_pos_req(&uri, 0, 11);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]), "Dirty overlay must return empty list");

    // Clean overlay on missing position
    overlay_mgr.close_document(file_str);
    let oob_req = make_pos_req(&uri, 99, 99);
    let oob_res = handle_implementation(&oob_req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(oob_res, json!([]));

    // Missing workspace
    let no_ws = handle_implementation(&req, None, &overlay_mgr, &mut conv);
    assert_eq!(no_ws, json!([]));
}
