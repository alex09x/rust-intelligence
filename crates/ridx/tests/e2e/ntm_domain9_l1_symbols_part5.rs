/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/documentSymbol part 5:
//! static items, functions, parameters, local let bindings, and modules.

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
fn test_document_symbol_static_item_name_kind() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/static_test.rs";
    let file = "/workspace/src/static_test.rs";
    let code = "pub static GLOBAL_FLAG: bool = true;\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "GLOBAL_FLAG");
    assert_eq!(items[0]["kind"], 13); // Variable / Static
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[0]["range"]["start"]["character"], 11);
    assert_eq!(items[0]["range"]["end"]["character"], 22);
}

#[test]
fn test_document_symbol_function_zero_params_no_return() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/fn_test.rs";
    let file = "/workspace/src/fn_test.rs";
    let code = "fn ping() {}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "ping");
    assert_eq!(items[0]["kind"], 12); // Function
    assert_eq!(items[0]["detail"], "fn ping()");
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[0]["range"]["start"]["character"], 3);
    assert_eq!(items[0]["range"]["end"]["character"], 7);
}

#[test]
fn test_document_symbol_function_with_return_type() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/fn_ret.rs";
    let file = "/workspace/src/fn_ret.rs";
    let code = "pub fn compute_pi() -> f64 { 3.14 }\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "compute_pi");
    assert_eq!(items[0]["kind"], 12);
    assert_eq!(items[0]["detail"], "fn compute_pi() -> f64");
}

#[test]
fn test_document_symbol_function_with_single_param() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/fn_param.rs";
    let file = "/workspace/src/fn_param.rs";
    let code = "fn greet(name: &str) {}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 2);

    let fn_sym = items.iter().find(|s| s["name"] == "greet").unwrap();
    assert_eq!(fn_sym["kind"], 12);

    let p_sym = items.iter().find(|s| s["name"] == "name").unwrap();
    assert_eq!(p_sym["kind"], 13);
    assert_eq!(p_sym["detail"], "name: &str");
}

#[test]
fn test_document_symbol_function_with_mut_param() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/fn_mut_p.rs";
    let file = "/workspace/src/fn_mut_p.rs";
    let code = "fn step(mut counter: i32) {}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 2);

    let p_sym = items.iter().find(|s| s["name"] == "counter").unwrap();
    assert_eq!(p_sym["kind"], 13);
}

#[test]
fn test_document_symbol_function_with_local_let() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/fn_local.rs";
    let file = "/workspace/src/fn_local.rs";
    let code = "fn run() {\n    let total = 100;\n}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 2);

    let fn_sym = items.iter().find(|s| s["name"] == "run").unwrap();
    assert_eq!(fn_sym["kind"], 12);

    let let_sym = items.iter().find(|s| s["name"] == "total").unwrap();
    assert_eq!(let_sym["kind"], 13);
    assert_eq!(let_sym["range"]["start"]["line"], 1);
}

#[test]
fn test_document_symbol_function_with_multiple_lets() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/fn_multi_let.rs";
    let file = "/workspace/src/fn_multi_let.rs";
    let code = "fn run() {\n    let a = 1;\n    let b = 2;\n}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 3);

    let a_sym = items.iter().find(|s| s["name"] == "a").unwrap();
    assert_eq!(a_sym["kind"], 13);

    let b_sym = items.iter().find(|s| s["name"] == "b").unwrap();
    assert_eq!(b_sym["kind"], 13);
}

#[test]
fn test_document_symbol_module_declaration() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/mod_decl.rs";
    let file = "/workspace/src/mod_decl.rs";
    let code = "pub mod parser;\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "parser");
    assert_eq!(items[0]["kind"], 2); // Module
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[0]["range"]["start"]["character"], 8);
    assert_eq!(items[0]["range"]["end"]["character"], 14);
}

#[test]
fn test_document_symbol_nested_submodule() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/nested_mods.rs";
    let file = "/workspace/src/nested_mods.rs";
    let code = "mod outer {\n    mod inner {}\n}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 2);

    let outer_sym = items.iter().find(|s| s["name"] == "outer").unwrap();
    assert_eq!(outer_sym["kind"], 2);

    let inner_sym = items.iter().find(|s| s["name"] == "inner").unwrap();
    assert_eq!(inner_sym["kind"], 2);
}
