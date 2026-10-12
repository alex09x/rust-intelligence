/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/documentSymbol part 4:
//! struct declarations, enum declarations, traits, type aliases, and const items.

use ridx::lsp::protocol::PositionConverter;
use ridx::lsp::symbols::handle_document_symbol;
use ridx::overlay::OverlayManager;
use serde_json::{json, Value};

fn make_doc_req(uri: &str) -> Value {
    json!({
        "params": {
            "textDocument": { "uri": uri }
        }
    })
}

#[test]
fn test_document_symbol_single_struct_name_kind_range() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/struct_test.rs";
    let file = "/workspace/src/struct_test.rs";
    let code = "pub struct UserProfile;\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "UserProfile");
    assert_eq!(items[0]["kind"], 23); // Struct
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[0]["range"]["start"]["character"], 11);
    assert_eq!(items[0]["range"]["end"]["character"], 22);
}

#[test]
fn test_document_symbol_multiple_structs_in_order() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/structs.rs";
    let file = "/workspace/src/structs.rs";
    let code = "struct Alpha;\nstruct Beta;\nstruct Gamma;\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["name"], "Alpha");
    assert_eq!(items[0]["kind"], 23);
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[1]["name"], "Beta");
    assert_eq!(items[1]["kind"], 23);
    assert_eq!(items[1]["range"]["start"]["line"], 1);
    assert_eq!(items[2]["name"], "Gamma");
    assert_eq!(items[2]["kind"], 23);
    assert_eq!(items[2]["range"]["start"]["line"], 2);
}

#[test]
fn test_document_symbol_single_enum_with_variants() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/enum_test.rs";
    let file = "/workspace/src/enum_test.rs";
    let code = "pub enum Color {\n    Red,\n    Green,\n    Blue,\n}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Color");
    assert_eq!(items[0]["kind"], 10); // Enum
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[0]["range"]["start"]["character"], 9);
    assert_eq!(items[0]["range"]["end"]["character"], 14);
}

#[test]
fn test_document_symbol_multiple_enums_in_file() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/enums.rs";
    let file = "/workspace/src/enums.rs";
    let code = "enum Direction { North, South }\nenum Speed { Fast, Slow }\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["name"], "Direction");
    assert_eq!(items[0]["kind"], 10);
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[1]["name"], "Speed");
    assert_eq!(items[1]["kind"], 10);
    assert_eq!(items[1]["range"]["start"]["line"], 1);
}

#[test]
fn test_document_symbol_single_trait_name_kind() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/trait_test.rs";
    let file = "/workspace/src/trait_test.rs";
    let code = "pub trait Renderable {}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Renderable");
    assert_eq!(items[0]["kind"], 11); // Trait
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[0]["range"]["start"]["character"], 10);
    assert_eq!(items[0]["range"]["end"]["character"], 20);
}

#[test]
fn test_document_symbol_multiple_traits_in_file() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/traits.rs";
    let file = "/workspace/src/traits.rs";
    let code = "pub trait Readable {}\npub trait Writable {}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["name"], "Readable");
    assert_eq!(items[0]["kind"], 11);
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[1]["name"], "Writable");
    assert_eq!(items[1]["kind"], 11);
    assert_eq!(items[1]["range"]["start"]["line"], 1);
}

#[test]
fn test_document_symbol_type_alias_name_kind() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/alias_test.rs";
    let file = "/workspace/src/alias_test.rs";
    let code = "pub type Coordinate = (f64, f64);\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Coordinate");
    assert_eq!(items[0]["kind"], 26); // TypeParameter / TyAlias
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[0]["range"]["start"]["character"], 9);
    assert_eq!(items[0]["range"]["end"]["character"], 19);
}

#[test]
fn test_document_symbol_multiple_type_aliases() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/aliases.rs";
    let file = "/workspace/src/aliases.rs";
    let code = "type Kilometers = f64;\ntype Miles = f64;\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["name"], "Kilometers");
    assert_eq!(items[0]["kind"], 26);
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[1]["name"], "Miles");
    assert_eq!(items[1]["kind"], 26);
    assert_eq!(items[1]["range"]["start"]["line"], 1);
}

#[test]
fn test_document_symbol_const_item_name_kind() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/const_test.rs";
    let file = "/workspace/src/const_test.rs";
    let code = "pub const BUFFER_SIZE: usize = 4096;\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "BUFFER_SIZE");
    assert_eq!(items[0]["kind"], 14); // Constant
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[0]["range"]["start"]["character"], 10);
    assert_eq!(items[0]["range"]["end"]["character"], 21);
}
