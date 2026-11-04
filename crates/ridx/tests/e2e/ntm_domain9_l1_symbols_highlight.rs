/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Hermetic integration tests for Domain 9: textDocument/documentHighlight and textDocument/documentSymbol.

use ridx::lsp::navigation::handle_document_highlight;
use ridx::lsp::protocol::PositionConverter;
use ridx::lsp::symbols::handle_document_symbol;
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

fn make_doc_req(uri: &str) -> Value {
    json!({
        "params": {
            "textDocument": { "uri": uri }
        }
    })
}

#[test]
fn test_l1_document_highlight_local_binding() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_hl", "0.1.0").unwrap();
    let uri = ws.file_uri("pkg_hl", "src/lib.rs").unwrap_or_else(|| {
        let p = ws.add_source_file("pkg_hl", "src/lib.rs", "").unwrap();
        format!("file://{}", p.display())
    });
    let file_path = ws.file_path("pkg_hl", "src/lib.rs").unwrap();
    let file_str = file_path.to_str().unwrap();

    let code = "pub fn compute() {\n    let mut total = 0;\n    total += 10;\n    println!(\"{}\", total);\n}\n";
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    overlay_mgr.open_document(&uri, file_str, 1, code.to_string());

    // Cursor on `total` declaration (line 1, col 12)
    let req = make_pos_req(&uri, 1, 12);
    let res = handle_document_highlight(&req, None, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("highlight items");
    assert_eq!(items.len(), 3, "Must highlight declaration and 2 usages");

    // First highlight is declaration / write (kind 3)
    assert_eq!(items[0]["kind"], 3);
    assert_eq!(items[0]["range"]["start"]["line"], 1);

    // Subsequent highlights are read / text (kind 1)
    // current behaviour: Text (1); Write (3) pending
    assert_eq!(items[1]["kind"], 1);
    assert_eq!(items[1]["range"]["start"]["line"], 2);
    assert_eq!(items[2]["kind"], 1);
    assert_eq!(items[2]["range"]["start"]["line"], 3);
}

#[test]
fn test_l1_document_highlight_workspace_references() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_hl_ws", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_hl_ws", "src/lib.rs", "pub fn «helper»() {}\n", DefKind::Fn).unwrap();

    ws.add_refs_to_def("pkg_hl_ws", "src/lib.rs", "pub fn helper() {}\npub fn run() {\n    «helper»();\n    «helper»();\n}\n", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_hl_ws").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_hl_ws", "src/lib.rs").unwrap();

    // Query on definition `helper` at line 0, col 7
    let req = make_pos_req(&uri, 0, 7);
    let res = handle_document_highlight(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().expect("highlight array");
    assert_eq!(items.len(), 3, "Must return exactly 3 highlights: 1 definition and 2 local references");
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[1]["range"]["start"]["line"], 2);
    assert_eq!(items[2]["range"]["start"]["line"], 3);
    // LSP DocumentHighlight is document-scoped (no URI field in LSP spec)
    assert!(items.iter().all(|item| item.get("uri").is_none()));
}

#[test]
fn test_l1_document_highlight_cross_file_excluded() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_hl_cross", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_hl_cross", "src/callee.rs", "pub fn «action»() {}\n", DefKind::Fn).unwrap();

    // Local call in callee.rs
    ws.add_refs_to_def("pkg_hl_cross", "src/callee.rs", "pub fn action() {}\npub fn local_invoke() { «action»(); }\n", ids[0]).unwrap();

    // Cross-file call in caller.rs
    ws.add_refs_to_def("pkg_hl_cross", "src/caller.rs", "use crate::callee::action;\npub fn remote_invoke() { «action»(); }\n", ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_hl_cross").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let callee_uri = ws.file_uri("pkg_hl_cross", "src/callee.rs").unwrap();

    // Query in callee.rs must only return highlights in callee.rs (definition and local call), excluding remote caller.rs
    let req = make_pos_req(&callee_uri, 0, 7);
    let res = handle_document_highlight(&req, Some(&index), &overlay_mgr, &mut conv);
    let items = res.as_array().expect("highlight items");
    assert_eq!(items.len(), 2, "Must return exactly 2 highlights within callee.rs (def and local call), excluding caller.rs");
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[1]["range"]["start"]["line"], 1);
    assert!(items.iter().all(|item| item.get("uri").is_none()));

    // Verify querying from caller.rs only returns caller.rs highlights
    let caller_uri = ws.file_uri("pkg_hl_cross", "src/caller.rs").unwrap();
    let caller_req = make_pos_req(&caller_uri, 1, 25);
    let caller_res = handle_document_highlight(&caller_req, Some(&index), &overlay_mgr, &mut conv);
    let caller_items = caller_res.as_array().expect("caller highlight items");
    assert_eq!(caller_items.len(), 1, "Query in caller.rs must only return its own reference site");
    assert_eq!(caller_items[0]["range"]["start"]["line"], 1);
    assert!(caller_items.iter().all(|item| item.get("uri").is_none()));
}

#[test]
fn test_l1_document_highlight_boundaries_and_empty() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_hl_bounds", "0.1.0").unwrap();
    let uri = ws.add_source_file("pkg_hl_bounds", "src/lib.rs", "pub fn empty() {}\n").map(|p| format!("file://{}", p.display())).unwrap();

    let shard = ws.build_crate_index("pkg_hl_bounds").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Whitespace
    let ws_req = make_pos_req(&uri, 0, 0);
    let ws_res = handle_document_highlight(&ws_req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(ws_res, json!([]));

    // Out of bounds
    let oob_req = make_pos_req(&uri, 999, 999);
    let oob_res = handle_document_highlight(&oob_req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(oob_res, json!([]));

    // Missing workspace
    let no_ws = handle_document_highlight(&ws_req, None, &overlay_mgr, &mut conv);
    assert_eq!(no_ws, json!([]));
}

#[test]
fn test_l1_document_symbol_all_kinds() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let uri = "file:///workspace/src/lib.rs";
    let file = "/workspace/src/lib.rs";
    let code = "\
pub struct Point { pub x: i32, pub y: i32 }
pub enum Direction { North, South, East, West }
pub fn calculate() -> i32 { 42 }
pub trait Renderable {}
pub const MAX_LIMIT: u32 = 100;
";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("document symbols list");

    assert_eq!(items.len(), 5, "Must discover all 5 symbols");

    let point = items.iter().find(|s| s["name"] == "Point").unwrap();
    assert_eq!(point["kind"], 23); // Struct

    let direction = items.iter().find(|s| s["name"] == "Direction").unwrap();
    assert_eq!(direction["kind"], 10); // Enum

    let calculate = items.iter().find(|s| s["name"] == "calculate").unwrap();
    assert_eq!(calculate["kind"], 12); // Function

    let renderable = items.iter().find(|s| s["name"] == "Renderable").unwrap();
    assert_eq!(renderable["kind"], 11); // Interface / Trait

    let max_limit = items.iter().find(|s| s["name"] == "MAX_LIMIT").unwrap();
    assert_eq!(max_limit["kind"], 14); // Constant
}

#[test]
fn test_l1_document_symbol_typaram_filtered() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let uri = "file:///workspace/src/generics.rs";
    let file = "/workspace/src/generics.rs";
    let code = "pub fn process_data<T>(val: T) -> T { val }\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("document symbols");

    // As in rust-analyzer: the function alone, no type parameter or value parameter symbols; generics stay in the detail.
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "process_data");
    assert_eq!(items[0]["kind"], 12);
    assert_eq!(items[0]["detail"], "fn<T>(val: T) -> T");
    assert_eq!(items[0]["children"], json!([]));
}

#[test]
fn test_l1_document_symbol_unopened_and_empty() {
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Document not open in overlay returns empty array
    let req = make_doc_req("file:///unopened/src/lib.rs");
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]));

    // Empty document in overlay returns empty array
    let mut mgr = OverlayManager::new();
    let empty_uri = "file:///workspace/src/empty.rs";
    mgr.open_document(empty_uri, "/workspace/src/empty.rs", 1, "".to_string());
    let empty_req = make_doc_req(empty_uri);
    let empty_res = handle_document_symbol(&empty_req, &mgr, &mut conv);
    assert_eq!(empty_res, json!([]));
}

#[test]
fn test_l1_document_symbol_unicode_ranges() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let uri = "file:///workspace/src/unicode.rs";
    let file = "/workspace/src/unicode.rs";
    // 🦀 takes 2 UTF-16 code units (surrogate pair)
    let code = "/* 🦀🦀 */ pub fn unicode_fn() {}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbols list");

    assert_eq!(items.len(), 1);
    let sym = &items[0];
    assert_eq!(sym["name"], "unicode_fn");
    // The item range starts at `pub`: "/* 🦀🦀 */ " = 3 + 4 + 4 = 11 UTF-16 code units.
    assert_eq!(sym["range"]["start"]["line"], 0);
    assert_eq!(sym["range"]["start"]["character"], 11);
    assert_eq!(sym["range"]["end"]["character"], 33);
    // "/* 🦀🦀 */ pub fn " = 3 + 4 + 11 = 18 UTF-16 code units; unicode_fn is 10 chars = 28 end
    assert_eq!(sym["selectionRange"]["start"]["character"], 18);
    assert_eq!(sym["selectionRange"]["end"]["character"], 28);
}
