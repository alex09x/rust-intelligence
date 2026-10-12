/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/implementation part 5:
//! cross-file implementations in same crate, multi-impl blocks, and marker traits.

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
fn test_implementation_cross_file_struct_two_impl_files() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_i5_1", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_i5_1", "src/models.rs", "pub struct «SessionData»;\n", DefKind::Struct)
        .unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file_with_impl_refs("pkg_i5_1", "src/file_a.rs", "«impl SessionData» {}\n", "pkg_i5_1", ids[0]).unwrap();
    ws.add_source_file_with_impl_refs("pkg_i5_1", "src/file_b.rs", "«impl SessionData» {}\n", "pkg_i5_1", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_i5_1").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let models_uri = ws.file_uri("pkg_i5_1", "src/models.rs").unwrap();
    let file_a_uri = ws.file_uri("pkg_i5_1", "src/file_a.rs").unwrap();
    let file_b_uri = ws.file_uri("pkg_i5_1", "src/file_b.rs").unwrap();

    let req = make_pos_req(&models_uri, 0, 12);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    let mut uris: Vec<_> = arr.iter().map(|l| l["uri"].as_str().unwrap()).collect();
    uris.sort();
    let mut expected = vec![file_a_uri.as_str(), file_b_uri.as_str()];
    expected.sort();
    assert_eq!(uris, expected);
}

#[test]
fn test_implementation_cross_file_trait_three_implementors() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_i5_2", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_i5_2", "src/traits.rs", "pub trait «Lifecycle»;\n", DefKind::Trait)
        .unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file_with_impl_refs("pkg_i5_2", "src/impl_a.rs", "«impl Lifecycle for ServiceA» {}\n", "pkg_i5_2", ids[0]).unwrap();
    ws.add_source_file_with_impl_refs("pkg_i5_2", "src/impl_b.rs", "«impl Lifecycle for ServiceB» {}\n", "pkg_i5_2", ids[0]).unwrap();
    ws.add_source_file_with_impl_refs("pkg_i5_2", "src/impl_c.rs", "«impl Lifecycle for ServiceC» {}\n", "pkg_i5_2", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_i5_2").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let traits_uri = ws.file_uri("pkg_i5_2", "src/traits.rs").unwrap();

    let req = make_pos_req(&traits_uri, 0, 11);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 3);
}

#[test]
fn test_implementation_marker_trait_no_methods() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_i5_3", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_i5_3", "src/lib.rs", "pub trait «Marker»;\n", DefKind::Trait)
        .unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub trait Marker;\n«impl Marker for MyType» {}\n";
    ws.add_source_file_with_impl_refs("pkg_i5_3", "src/lib.rs", code, "pkg_i5_3", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_i5_3").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_i5_3", "src/lib.rs").unwrap();

    let req = make_pos_req(&uri, 0, 11);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["uri"], uri);
}

#[test]
fn test_implementation_generic_struct_concrete_impl() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_i5_4", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_i5_4", "src/lib.rs", "pub struct «Stack»<T>(pub Vec<T>);\n", DefKind::Struct)
        .unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub struct Stack<T>(pub Vec<T>);\n«impl Stack<i32>» {}\n";
    ws.add_source_file_with_impl_refs("pkg_i5_4", "src/lib.rs", code, "pkg_i5_4", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_i5_4").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_i5_4", "src/lib.rs").unwrap();

    let req = make_pos_req(&uri, 0, 12);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 1);
}

#[test]
fn test_implementation_generic_struct_generic_impl() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_i5_5", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_i5_5", "src/lib.rs", "pub struct «Queue»<T>(pub Vec<T>);\n", DefKind::Struct)
        .unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub struct Queue<T>(pub Vec<T>);\n«impl<T> Queue<T>» {}\n";
    ws.add_source_file_with_impl_refs("pkg_i5_5", "src/lib.rs", code, "pkg_i5_5", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_i5_5").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_i5_5", "src/lib.rs").unwrap();

    let req = make_pos_req(&uri, 0, 12);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 1);
}

#[test]
fn test_implementation_enum_with_multiple_trait_impls() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_i5_6", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_i5_6", "src/lib.rs", "pub enum «Operation» { Read, Write }\n", DefKind::Enum)
        .unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub enum Operation { Read, Write }\n«impl Operation» {}\n«impl Display for Operation» {}\n";
    ws.add_source_file_with_impl_refs("pkg_i5_6", "src/lib.rs", code, "pkg_i5_6", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_i5_6").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_i5_6", "src/lib.rs").unwrap();

    let req = make_pos_req(&uri, 0, 10);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 2);
}

#[test]
fn test_implementation_tuple_struct_impl() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_i5_7", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_i5_7", "src/lib.rs", "pub struct «Meters»(pub f64);\n", DefKind::Struct)
        .unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub struct Meters(pub f64);\n«impl Meters» {}\n";
    ws.add_source_file_with_impl_refs("pkg_i5_7", "src/lib.rs", code, "pkg_i5_7", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_i5_7").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_i5_7", "src/lib.rs").unwrap();

    let req = make_pos_req(&uri, 0, 12);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 1);
}

#[test]
fn test_implementation_unit_struct_impl() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_i5_8", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_i5_8", "src/lib.rs", "pub struct «UnitType»;\n", DefKind::Struct)
        .unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub struct UnitType;\n«impl UnitType» {}\n";
    ws.add_source_file_with_impl_refs("pkg_i5_8", "src/lib.rs", code, "pkg_i5_8", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_i5_8").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_i5_8", "src/lib.rs").unwrap();

    let req = make_pos_req(&uri, 0, 12);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 1);
}

#[test]
fn test_implementation_deep_module_tree_impl() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_i5_9", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs("pkg_i5_9", "src/a/b/c/mod.rs", "pub struct «DeepTreeStruct»;\n", DefKind::Struct)
        .unwrap();
    assert_eq!(ids.len(), 1);

    let code = "use crate::a::b::c::DeepTreeStruct;\n«impl DeepTreeStruct» {}\n";
    ws.add_source_file_with_impl_refs("pkg_i5_9", "src/x/y/z/mod.rs", code, "pkg_i5_9", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_i5_9").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let def_uri = ws.file_uri("pkg_i5_9", "src/a/b/c/mod.rs").unwrap();
    let impl_uri = ws.file_uri("pkg_i5_9", "src/x/y/z/mod.rs").unwrap();

    let req = make_pos_req(&def_uri, 0, 12);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["uri"], impl_uri);
}
