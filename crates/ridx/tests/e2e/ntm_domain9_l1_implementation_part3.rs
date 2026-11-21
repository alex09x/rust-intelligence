/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/implementation (Part 3):
//! cross-crate multi-shard, trait method cursor, generic impl, blanket impl,
//! dirty overlay, and UTF-16 surrogate pairs.

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

/// `loc` covers `start_col..end_col` of `line`: an impl's self type, where rust-analyzer places an implementation
/// (prod-code, m=20226).
fn assert_range(loc: &Value, line: u32, start_col: u32, end_col: u32) {
    assert_eq!(loc["range"]["start"]["line"], line);
    assert_eq!(loc["range"]["start"]["character"], start_col);
    assert_eq!(loc["range"]["end"]["line"], line);
    assert_eq!(loc["range"]["end"]["character"], end_col);
}

#[test]
fn test_l1_implementation_cross_crate_two_shards() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("crate_traits", "0.1.0").unwrap();
    ws.add_crate("crate_consumer", "0.1.0").unwrap();

    let (_, ids) = ws.add_source_file_with_defs("crate_traits", "src/lib.rs", "pub trait «Service»;\n", DefKind::Trait).unwrap();

    let local_code = "pub trait Service;\n«impl Service for LocalEngine» {}\n";
    ws.add_source_file_with_impl_refs("crate_traits", "src/lib.rs", local_code, "crate_traits", ids[0]).unwrap();

    let consumer_code = "use crate_traits::Service;\n«impl Service for RemoteEngine» {}\n";
    ws.add_source_file_with_impl_refs("crate_consumer", "src/lib.rs", consumer_code, "crate_traits", ids[0]).unwrap();

    let _ = ws.build_crate_index("crate_traits").unwrap();
    let _ = ws.build_crate_index("crate_consumer").unwrap();

    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    assert_eq!(index.readers().len(), 2, "WorkspaceIndex::open must load both crate shards");

    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let traits_uri = ws.file_uri("crate_traits", "src/lib.rs").unwrap();
    let consumer_uri = ws.file_uri("crate_consumer", "src/lib.rs").unwrap();

    let req = make_pos_req(&traits_uri, 0, 10);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().expect("impl list");

    let item_traits = items.iter().find(|it| it["uri"] == traits_uri).expect("traits impl");
    assert_range(item_traits, 1, 17, 28);

    let item_consumer = items.iter().find(|it| it["uri"] == consumer_uri).expect("consumer impl");
    assert_range(item_consumer, 1, 17, 29);
}

#[test]
fn test_l1_implementation_cursor_on_trait_method_returns_empty() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_method_impl", "0.1.0").unwrap();

    let text_trait = "pub trait «Greeter» {\n    fn hello(&self);\n}\nimpl Greeter for Console {\n    fn hello(&self) {}\n}\n";
    let text_method = "pub trait Greeter {\n    fn «hello»(&self);\n}\nimpl Greeter for Console {\n    fn hello(&self) {}\n}\n";
    let text_impl = "pub trait Greeter {\n    fn hello(&self);\n}\n«impl Greeter for Console» {\n    fn hello(&self) {}\n}\n";

    let (_, ids_trait) = ws.add_source_file_with_defs("pkg_method_impl", "src/lib.rs", text_trait, DefKind::Trait).unwrap();

    ws.add_source_file_with_defs("pkg_method_impl", "src/lib.rs", text_method, DefKind::AssocFn).unwrap();

    ws.add_source_file_with_impl_refs("pkg_method_impl", "src/lib.rs", text_impl, "pkg_method_impl", ids_trait[0]).unwrap();

    let shard = ws.build_crate_index("pkg_method_impl").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_method_impl", "src/lib.rs").unwrap();

    // 1. Query on method definition inside trait (line 1, col 7: `hello`)
    // Implementation query on an AssocFn definition must return empty list
    let req_method = make_pos_req(&uri, 1, 7);
    let res_method = handle_implementation(&req_method, Some(&index), &overlay_mgr, &mut conv);
    let items_method = res_method.as_array().expect("impl list");
    assert_eq!(items_method.len(), 0, "Cursor on trait method definition must return empty list");

    // 2. Query on trait definition itself (line 0, col 10: `Greeter`)
    // Verifies that the implementation exists in the index and the empty result above is not from an empty shard
    let req_trait = make_pos_req(&uri, 0, 10);
    let res_trait = handle_implementation(&req_trait, Some(&index), &overlay_mgr, &mut conv);
    let items_trait = res_trait.as_array().expect("impl list");
    assert_eq!(items_trait.len(), 1, "Cursor on trait definition must find impl in shard");
    assert_eq!(items_trait[0]["uri"], uri);
    assert_range(&items_trait[0], 3, 17, 24);
}

#[test]
fn test_l1_implementation_generic_impl_indexed_ref() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_generic_impl", "0.1.0").unwrap();

    let (_, ids) = ws.add_source_file_with_defs("pkg_generic_impl", "src/lib.rs", "pub trait «Marker»;\n", DefKind::Trait).unwrap();

    let code = "pub trait Marker;\n«impl<T> Marker for T» {}\n";
    ws.add_source_file_with_impl_refs("pkg_generic_impl", "src/lib.rs", code, "pkg_generic_impl", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_generic_impl").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_generic_impl", "src/lib.rs").unwrap();

    let req = make_pos_req(&uri, 0, 10);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().expect("impl list");

    assert_eq!(items.len(), 1, "Returns generic impl for T from indexed ref");
    assert_eq!(items[0]["uri"], uri);
    assert_range(&items[0], 1, 19, 20);
}

#[test]
fn test_l1_implementation_blanket_impl_indexed_ref() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_blanket_impl", "0.1.0").unwrap();

    let (_, ids) = ws.add_source_file_with_defs("pkg_blanket_impl", "src/lib.rs", "pub trait «Marker»;\n", DefKind::Trait).unwrap();

    let code = "pub trait Marker;\n«impl<T> Marker for &T» {}\n";
    ws.add_source_file_with_impl_refs("pkg_blanket_impl", "src/lib.rs", code, "pkg_blanket_impl", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_blanket_impl").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_blanket_impl", "src/lib.rs").unwrap();

    let req = make_pos_req(&uri, 0, 10);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().expect("impl list");

    assert_eq!(items.len(), 1, "Returns blanket impl for &T from indexed ref");
    assert_eq!(items[0]["uri"], uri);
    assert_range(&items[0], 1, 19, 21);
}

#[test]
fn test_l1_implementation_dirty_overlay_returns_empty() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_dirty_impl", "0.1.0").unwrap();

    let (_, ids) = ws.add_source_file_with_defs("pkg_dirty_impl", "src/lib.rs", "pub trait «Handler»;\n", DefKind::Trait).unwrap();

    let code = "pub trait Handler;\n«impl Handler for Worker» {}\n";
    ws.add_source_file_with_impl_refs("pkg_dirty_impl", "src/lib.rs", code, "pkg_dirty_impl", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_dirty_impl").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_dirty_impl", "src/lib.rs").unwrap();
    let file = ws.file_path("pkg_dirty_impl", "src/lib.rs").unwrap();
    let file_str = file.to_str().unwrap();

    overlay_mgr.open_document(&uri, file_str, 1, code.to_string());
    overlay_mgr.update(file_str, 2, "// dirty edit\n".to_string());

    let req = make_pos_req(&uri, 0, 10);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().expect("impl list");
    assert_eq!(items.len(), 0, "Dirty overlay on queried file must return empty list");
}

#[test]
fn test_l1_implementation_utf16_multibyte_prefix_columns() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_utf16_impl", "0.1.0").unwrap();

    let (_, ids) = ws.add_source_file_with_defs("pkg_utf16_impl", "src/lib.rs", "pub struct «Engine»;\n", DefKind::Struct).unwrap();

    // 4-byte UTF-8 emoji (\u{1F680}) has 2 UTF-16 code units.
    // "/* " (3) + 1 emoji (2 units) + " */ " (4) = 9 UTF-16 code units prefix.
    let code = "\
pub struct Engine;
/* \u{1F680} */ «impl Worker for Engine» {}
/* \u{1F680} */ «impl Runner for Engine» {}
";
    ws.add_source_file_with_impl_refs("pkg_utf16_impl", "src/lib.rs", code, "pkg_utf16_impl", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_utf16_impl").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_utf16_impl", "src/lib.rs").unwrap();

    let req = make_pos_req(&uri, 0, 11);
    let res = handle_implementation(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().expect("impl list");

    assert_eq!(items.len(), 2, "Must find both implementations");
    assert_eq!(items[0]["uri"], uri);
    assert_range(&items[0], 1, 25, 31);

    assert_eq!(items[1]["uri"], uri);
    assert_range(&items[1], 2, 25, 31);
}

/// Review of 465e02ad: the self type was read from the disk while the index and a clean buffer had another header.
#[test]
fn test_l1_implementation_self_type_is_read_from_the_open_buffer() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_buffer_impl", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_buffer_impl", "src/lib.rs", "pub trait «Handler»;\n", DefKind::Trait).unwrap();
    let code = "pub trait Handler;\n«impl Handler for Worker» {}\n";
    ws.add_source_file_with_impl_refs("pkg_buffer_impl", "src/lib.rs", code, "pkg_buffer_impl", ids[0]).unwrap();
    let shard = ws.build_crate_index("pkg_buffer_impl").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let uri = ws.file_uri("pkg_buffer_impl", "src/lib.rs").unwrap();
    let file = ws.file_path("pkg_buffer_impl", "src/lib.rs").unwrap();
    let file_str = file.to_str().unwrap();
    let mut overlay_mgr = OverlayManager::new();
    overlay_mgr.open_document(&uri, file_str, 1, "pub trait Handler;\nimpl Handler for Worker {}\n".to_string());
    std::fs::write(&file, "pub trait Handler;\nimpl Handler for LongerName {}\n").unwrap();
    let mut conv = PositionConverter::new();
    let res = handle_implementation(&make_pos_req(&uri, 0, 10), Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().expect("impl list");
    assert_eq!(items.len(), 1);
    assert_range(&items[0], 1, 17, 23);
}
