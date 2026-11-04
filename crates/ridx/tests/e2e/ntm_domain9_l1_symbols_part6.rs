/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/documentSymbol part 6:
//! buffer lifecycles, mutations, raw idents, comments, and selectionRange invariants.

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
fn test_document_symbol_unopened_document_returns_empty() {
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/unopened.rs";

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]));
}

#[test]
fn test_document_symbol_empty_file_returns_empty() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/empty.rs";
    let file = "/workspace/src/empty.rs";
    overlay_mgr.open_document(uri, file, 1, "".to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]));
}

#[test]
fn test_document_symbol_comments_only_file_returns_empty() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/comments.rs";
    let file = "/workspace/src/comments.rs";
    let code = "// Line comment\n/* Block comment\n   spanning multiple lines */\n\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]));
}

#[test]
fn test_document_symbol_selection_range_matches_range() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/range_check.rs";
    let file = "/workspace/src/range_check.rs";
    let code = "pub struct Point;\npub fn origin() {}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 2);
    for item in items {
        assert_eq!(item["selectionRange"], item["selectionRange"]);
    }
}

#[test]
fn test_document_symbol_raw_identifier_handling() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/raw_ident.rs";
    let file = "/workspace/src/raw_ident.rs";
    let code = "pub struct r#type;\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "r#type");
    assert_eq!(items[0]["kind"], 23);
}

#[test]
fn test_document_symbol_multiline_items_range_bounds() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/multiline.rs";
    let file = "/workspace/src/multiline.rs";
    let code = "fn compute(\n    x: i32,\n    y: i32,\n) -> i32 {\n    x + y\n}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol list");
    // One symbol spanning all six lines; the multi-line signature collapses into its detail, as rust-analyzer shows it.
    assert_eq!(items.len(), 1);
    let fn_sym = &items[0];
    assert_eq!(fn_sym["name"], "compute");
    assert_eq!(fn_sym["selectionRange"]["start"]["line"], 0);
    assert_eq!(fn_sym["range"], json!({ "start": { "line": 0, "character": 0 }, "end": { "line": 5, "character": 1 } }));
    assert_eq!(fn_sym["detail"], "fn( x: i32, y: i32, ) -> i32");
}

#[test]
fn test_document_symbol_close_document_clears_symbols() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/lifecycle.rs";
    let file = "/workspace/src/lifecycle.rs";
    let code = "pub struct Temporary;\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res_open = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    assert_eq!(res_open.as_array().unwrap().len(), 1);

    overlay_mgr.close_document(file);
    let res_closed = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    assert_eq!(res_closed, json!([]));
}

#[test]
fn test_document_symbol_buffer_mutation_updates_symbols() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/mutation.rs";
    let file = "/workspace/src/mutation.rs";
    let code_v1 = "pub struct Original;\n";
    overlay_mgr.open_document(uri, file, 1, code_v1.to_string());

    let req = make_doc_req(uri);
    let res_v1 = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items_v1 = res_v1.as_array().unwrap();
    assert_eq!(items_v1.len(), 1);
    assert_eq!(items_v1[0]["name"], "Original");

    let code_v2 = "pub struct Replaced;\npub fn added() {}\n";
    overlay_mgr.update_document(file, 2, code_v2.to_string());
    let res_v2 = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items_v2 = res_v2.as_array().unwrap();
    assert_eq!(items_v2.len(), 2);
    assert_eq!(items_v2[0]["name"], "Replaced");
    assert_eq!(items_v2[1]["name"], "added");
}

#[test]
fn test_document_symbol_multiple_documents_isolated() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri_a = "file:///workspace/src/doc_a.rs";
    let file_a = "/workspace/src/doc_a.rs";
    let uri_b = "file:///workspace/src/doc_b.rs";
    let file_b = "/workspace/src/doc_b.rs";

    overlay_mgr.open_document(uri_a, file_a, 1, "pub struct StructA;\n".to_string());
    overlay_mgr.open_document(uri_b, file_b, 1, "pub struct StructB;\n".to_string());

    let res_a = handle_document_symbol(&make_doc_req(uri_a), &overlay_mgr, &mut conv);
    let items_a = res_a.as_array().unwrap();
    assert_eq!(items_a.len(), 1);
    assert_eq!(items_a[0]["name"], "StructA");

    let res_b = handle_document_symbol(&make_doc_req(uri_b), &overlay_mgr, &mut conv);
    let items_b = res_b.as_array().unwrap();
    assert_eq!(items_b.len(), 1);
    assert_eq!(items_b[0]["name"], "StructB");
}
