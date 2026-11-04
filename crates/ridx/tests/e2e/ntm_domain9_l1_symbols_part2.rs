/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Hermetic integration tests for Domain 9: documentHighlight and documentSymbol (part 2).

use ridx::lsp::navigation::handle_document_highlight;
use ridx::lsp::protocol::PositionConverter;
use ridx::lsp::symbols::handle_document_symbol;
use ridx::overlay::OverlayManager;
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

fn make_doc_req(uri: &str) -> Value {
    json!({
        "params": {
            "textDocument": { "uri": uri }
        }
    })
}

#[test]
fn test_l1_document_symbol_nested_methods() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let uri = "file:///workspace/src/methods.rs";
    let file = "/workspace/src/methods.rs";
    let code = "\
pub struct Widget;
impl Widget {
    pub fn new() -> Self { Widget }
    pub fn execute(&self) -> i32 { 100 }
}
";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol list");

    // As in rust-analyzer: the struct and its impl block, the methods nested in the impl.
    let names: Vec<&str> = items.iter().filter_map(|s| s["name"].as_str()).collect();
    assert_eq!(names, vec!["Widget", "impl Widget"], "Must discover struct and impl");

    let widget = items.iter().find(|s| s["name"] == "Widget").unwrap();
    assert_eq!(widget["kind"], 23); // Struct
    assert_eq!(widget["selectionRange"]["start"]["line"], 0);
    assert_eq!(widget["selectionRange"]["start"]["character"], 11);
    assert_eq!(widget["selectionRange"]["end"]["character"], 17);

    let imp = items.iter().find(|s| s["name"] == "impl Widget").unwrap();
    assert_eq!(imp["kind"], 19); // Object
    let new_fn = &imp["children"][0];
    assert_eq!(new_fn["name"], "new");
    assert_eq!(new_fn["kind"], 12); // Function: no `self`
    assert_eq!(new_fn["selectionRange"]["start"]["line"], 2);
    assert_eq!(new_fn["selectionRange"]["start"]["character"], 11);
    assert_eq!(new_fn["selectionRange"]["end"]["character"], 14);

    let exec = &imp["children"][1];
    assert_eq!(exec["name"], "execute");
    assert_eq!(exec["kind"], 6); // Method: takes `&self`
    assert_eq!(exec["selectionRange"]["start"]["line"], 3);
    assert_eq!(exec["selectionRange"]["start"]["character"], 11);
    assert_eq!(exec["selectionRange"]["end"]["character"], 18);
}

#[test]
fn test_l1_document_symbol_crlf_preservation() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let uri = "file:///workspace/src/crlf.rs";
    let file = "/workspace/src/crlf.rs";
    let code = "pub fn first() {}\r\npub fn second() {}\r\npub fn third() {}\r\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("crlf symbols");

    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["name"], "first");
    assert_eq!(items[0]["selectionRange"]["start"]["line"], 0);
    assert_eq!(items[0]["selectionRange"]["start"]["character"], 7);
    assert_eq!(items[0]["selectionRange"]["end"]["character"], 12);
    // The item range ends at its `}`, before the `\r\n`.
    assert_eq!(items[0]["range"]["end"], json!({ "line": 0, "character": 17 }));

    assert_eq!(items[1]["name"], "second");
    assert_eq!(items[1]["selectionRange"]["start"]["line"], 1);
    assert_eq!(items[1]["selectionRange"]["start"]["character"], 7);
    assert_eq!(items[1]["selectionRange"]["end"]["character"], 13);

    assert_eq!(items[2]["name"], "third");
    assert_eq!(items[2]["selectionRange"]["start"]["line"], 2);
    assert_eq!(items[2]["selectionRange"]["start"]["character"], 7);
    assert_eq!(items[2]["selectionRange"]["end"]["character"], 12);
}

#[test]
fn test_l1_document_symbol_selection_range_exact() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let uri = "file:///workspace/src/ranges.rs";
    let file = "/workspace/src/ranges.rs";
    let code = "pub fn calculate_sum(a: i32, b: i32) -> i32 { a + b }\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol list");

    // Parameters are no symbols; the function's range spans the item, its selection range the name.
    assert_eq!(items.len(), 1);
    let func = &items[0];
    assert_eq!(func["name"], "calculate_sum");
    assert_eq!(func["kind"], 12);
    assert_eq!(func["selectionRange"], json!({ "start": { "line": 0, "character": 7 }, "end": { "line": 0, "character": 20 } }));
    assert_eq!(func["range"], json!({ "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 53 } }));
    assert_eq!(func["detail"], "fn(a: i32, b: i32) -> i32");
}

#[test]
fn test_l1_document_highlight_same_line_multiple_occurrences() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_hl_sameline", "0.1.0").unwrap();
    let p = ws.add_source_file("pkg_hl_sameline", "src/lib.rs", "").unwrap();
    let uri = ws.file_uri("pkg_hl_sameline", "src/lib.rs").unwrap();
    let file_str = p.to_str().unwrap();

    let code = "pub fn test() {\n    let val = 10;\n    let sum = val + val;\n}\n";
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    overlay_mgr.open_document(&uri, file_str, 1, code.to_string());

    // Cursor on `val` declaration (line 1, col 9)
    let req = make_pos_req(&uri, 1, 9);
    let res = handle_document_highlight(&req, None, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("highlights");

    assert_eq!(items.len(), 3, "Must highlight 1 declaration and 2 usages on same line");
    assert_eq!(items[0]["range"]["start"]["line"], 1);
    assert_eq!(items[0]["range"]["start"]["character"], 8);
    assert_eq!(items[0]["range"]["end"]["character"], 11);

    assert_eq!(items[1]["range"]["start"]["line"], 2);
    assert_eq!(items[1]["range"]["start"]["character"], 14);
    assert_eq!(items[1]["range"]["end"]["character"], 17);

    assert_eq!(items[2]["range"]["start"]["line"], 2);
    assert_eq!(items[2]["range"]["start"]["character"], 20);
    assert_eq!(items[2]["range"]["end"]["character"], 23);
}

#[test]
fn test_l1_document_highlight_shadowed_local_binding() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_hl_shadow", "0.1.0").unwrap();
    let p = ws.add_source_file("pkg_hl_shadow", "src/lib.rs", "").unwrap();
    let uri = ws.file_uri("pkg_hl_shadow", "src/lib.rs").unwrap();
    let file_str = p.to_str().unwrap();

    let code = "pub fn test_shadow() {\n    let x = 1;\n    {\n        let x = 2;\n        println!(\"{}\", x);\n    }\n    println!(\"{}\", x);\n}\n";
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    overlay_mgr.open_document(&uri, file_str, 1, code.to_string());

    // Cursor on inner `x` declaration at line 3, col 12
    let req = make_pos_req(&uri, 3, 12);
    let res = handle_document_highlight(&req, None, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("shadowed highlights");

    // Must only highlight inner scope x (lines 3 and 4), NOT outer scope (lines 1 and 6)
    assert_eq!(items.len(), 2, "Shadowed binding must only highlight inner scope");
    assert_eq!(items[0]["range"]["start"]["line"], 3);
    assert_eq!(items[0]["range"]["start"]["character"], 12);
    assert_eq!(items[0]["range"]["end"]["character"], 13);

    assert_eq!(items[1]["range"]["start"]["line"], 4);
    assert_eq!(items[1]["range"]["start"]["character"], 23);
    assert_eq!(items[1]["range"]["end"]["character"], 24);
}

#[test]
fn test_l1_document_highlight_cursor_at_end_col() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_hl_endcol", "0.1.0").unwrap();
    let p = ws.add_source_file("pkg_hl_endcol", "src/lib.rs", "").unwrap();
    let uri = ws.file_uri("pkg_hl_endcol", "src/lib.rs").unwrap();
    let file_str = p.to_str().unwrap();

    let code = "pub fn run() {\n    let item_counter = 42;\n    println!(\"{}\", item_counter);\n}\n";
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    overlay_mgr.open_document(&uri, file_str, 1, code.to_string());

    // Cursor at the end column of `item_counter` (line 1, col 20)
    let req = make_pos_req(&uri, 1, 20);
    let res = handle_document_highlight(&req, None, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("highlights");

    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["range"]["start"]["line"], 1);
    assert_eq!(items[0]["range"]["start"]["character"], 8);
    assert_eq!(items[0]["range"]["end"]["character"], 20);

    assert_eq!(items[1]["range"]["start"]["line"], 2);
    assert_eq!(items[1]["range"]["start"]["character"], 19);
    assert_eq!(items[1]["range"]["end"]["character"], 31);
}

#[test]
fn test_l1_document_highlight_multi_line_references() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_hl_multiline", "0.1.0").unwrap();
    let p = ws.add_source_file("pkg_hl_multiline", "src/lib.rs", "").unwrap();
    let uri = ws.file_uri("pkg_hl_multiline", "src/lib.rs").unwrap();
    let file_str = p.to_str().unwrap();

    let code = concat!(
        "pub fn sequence() {\n",
        "    let mut count = 0;\n",
        "    count += 1;\n",
        "    if count > 0 {\n",
        "        count += 2;\n",
        "    }\n",
        "    println!(\"{}\", count);\n",
        "}\n",
    );
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    overlay_mgr.open_document(&uri, file_str, 1, code.to_string());

    let req = make_pos_req(&uri, 1, 13);
    let res = handle_document_highlight(&req, None, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("multi-line highlights");

    assert_eq!(items.len(), 5, "Must find declaration and 4 references across lines");
    let ranges: Vec<(u64, u64, u64)> = items
        .iter()
        .map(|item| {
            let start = &item["range"]["start"];
            let end = &item["range"]["end"];
            (start["line"].as_u64().unwrap(), start["character"].as_u64().unwrap(), end["character"].as_u64().unwrap())
        })
        .collect();
    assert_eq!(ranges, vec![(1, 12, 17), (2, 4, 9), (3, 7, 12), (4, 8, 13), (6, 19, 24),]);
}
