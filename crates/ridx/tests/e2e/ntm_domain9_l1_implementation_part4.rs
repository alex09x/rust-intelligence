/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/implementation part 4:
//! trait implementations by various types, inherent impls, and generic impls.

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
fn test_implementation_trait_implemented_by_two_structs() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_i4_1", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_i4_1", "src/lib.rs", "pub trait «Renderable»;\n", DefKind::Trait).unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub trait Renderable;\n«impl Renderable for Button» {}\n«impl Renderable for Label» {}\n";
    ws.add_source_file_with_impl_refs("pkg_i4_1", "src/lib.rs", code, "pkg_i4_1", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_i4_1").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_i4_1", "src/lib.rs").unwrap();

    let req = make_pos_req(&uri, 0, 11);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["uri"], uri);
    assert_eq!(arr[1]["uri"], uri);
}

#[test]
fn test_implementation_trait_implemented_by_enum() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_i4_2", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_i4_2", "src/lib.rs", "pub trait «Parseable»;\n", DefKind::Trait).unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub trait Parseable;\n«impl Parseable for Token» {}\n";
    ws.add_source_file_with_impl_refs("pkg_i4_2", "src/lib.rs", code, "pkg_i4_2", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_i4_2").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_i4_2", "src/lib.rs").unwrap();

    let req = make_pos_req(&uri, 0, 11);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["uri"], uri);
}

#[test]
fn test_implementation_trait_implemented_by_primitive_wrapper() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_i4_3", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_i4_3", "src/lib.rs", "pub trait «ByteConvertible»;\n", DefKind::Trait).unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub trait ByteConvertible;\n«impl ByteConvertible for Wrapper» {}\n";
    ws.add_source_file_with_impl_refs("pkg_i4_3", "src/lib.rs", code, "pkg_i4_3", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_i4_3").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_i4_3", "src/lib.rs").unwrap();

    let req = make_pos_req(&uri, 0, 11);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 1);
}

#[test]
fn test_implementation_struct_with_inherent_impl_only() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_i4_4", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_i4_4", "src/lib.rs", "pub struct «ServiceInstance»;\n", DefKind::Struct).unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub struct ServiceInstance;\n«impl ServiceInstance» {}\n";
    ws.add_source_file_with_impl_refs("pkg_i4_4", "src/lib.rs", code, "pkg_i4_4", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_i4_4").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_i4_4", "src/lib.rs").unwrap();

    let req = make_pos_req(&uri, 0, 12);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["uri"], uri);
}

#[test]
fn test_implementation_struct_with_multiple_inherent_impl_blocks() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_i4_5", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_i4_5", "src/lib.rs", "pub struct «MultiBlockStruct»;\n", DefKind::Struct).unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub struct MultiBlockStruct;\n«impl MultiBlockStruct» {}\n«impl MultiBlockStruct» {}\n";
    ws.add_source_file_with_impl_refs("pkg_i4_5", "src/lib.rs", code, "pkg_i4_5", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_i4_5").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_i4_5", "src/lib.rs").unwrap();

    let req = make_pos_req(&uri, 0, 12);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 2);
}

#[test]
fn test_implementation_trait_with_generic_type_param_impl() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_i4_6", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_i4_6", "src/lib.rs", "pub trait «Transformer»;\n", DefKind::Trait).unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub trait Transformer;\n«impl Transformer for String» {}\n";
    ws.add_source_file_with_impl_refs("pkg_i4_6", "src/lib.rs", code, "pkg_i4_6", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_i4_6").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_i4_6", "src/lib.rs").unwrap();

    let req = make_pos_req(&uri, 0, 11);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 1);
}

#[test]
fn test_implementation_struct_impl_in_submodule() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_i4_7", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_i4_7", "src/models.rs", "pub struct «Widget»;\n", DefKind::Struct).unwrap();
    assert_eq!(ids.len(), 1);

    let code = "use crate::models::Widget;\n«impl Widget» {}\n";
    ws.add_source_file_with_impl_refs("pkg_i4_7", "src/sub/impls.rs", code, "pkg_i4_7", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_i4_7").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let models_uri = ws.file_uri("pkg_i4_7", "src/models.rs").unwrap();
    let impls_uri = ws.file_uri("pkg_i4_7", "src/sub/impls.rs").unwrap();

    let req = make_pos_req(&models_uri, 0, 12);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["uri"], impls_uri);
}

#[test]
fn test_implementation_trait_impl_in_parent_module() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_i4_8", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_i4_8", "src/sub/traits.rs", "pub trait «Emitter»;\n", DefKind::Trait).unwrap();
    assert_eq!(ids.len(), 1);

    let code = "«impl Emitter for Console» {}\n";
    ws.add_source_file_with_impl_refs("pkg_i4_8", "src/lib.rs", code, "pkg_i4_8", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_i4_8").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let traits_uri = ws.file_uri("pkg_i4_8", "src/sub/traits.rs").unwrap();
    let lib_uri = ws.file_uri("pkg_i4_8", "src/lib.rs").unwrap();

    let req = make_pos_req(&traits_uri, 0, 11);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["uri"], lib_uri);
}

#[test]
fn test_implementation_struct_impl_with_where_clause() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_i4_9", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_i4_9", "src/lib.rs", "pub struct «Container»;\n", DefKind::Struct).unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub struct Container;\n«impl Container» {}\n";
    ws.add_source_file_with_impl_refs("pkg_i4_9", "src/lib.rs", code, "pkg_i4_9", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_i4_9").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_i4_9", "src/lib.rs").unwrap();

    let req = make_pos_req(&uri, 0, 12);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["uri"], uri);
}
