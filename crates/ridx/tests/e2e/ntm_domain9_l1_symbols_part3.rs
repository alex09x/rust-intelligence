/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/documentSymbol (part 3):
//! signatures, nesting invariants, enums, modules, constants, traits, and buffer updates.

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
fn test_l1_document_symbol_function_signature_detail_exact() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/math.rs";
    let file = "/workspace/src/math.rs";
    let code = "pub fn add(a: i32, b: i32) -> i32 { a + b }\npub struct Config;\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol array");
    // Discovers top-level items and function parameters as local symbols
    assert_eq!(items.len(), 4);

    let add_sym = items.iter().find(|s| s["name"] == "add").unwrap();
    assert_eq!(add_sym["kind"], 12); // Function
    assert_eq!(add_sym["detail"], "fn add(a: i32, b: i32) -> i32");
    assert_eq!(add_sym["range"]["start"]["line"], 0);
    assert_eq!(add_sym["range"]["start"]["character"], 7);
    assert_eq!(add_sym["range"]["end"]["character"], 10);

    let param_a = items.iter().find(|s| s["name"] == "a").unwrap();
    assert_eq!(param_a["kind"], 13); // Variable / Local parameter
    assert_eq!(param_a["detail"], "a: i32");
    assert_eq!(param_a["range"]["start"]["line"], 0);
    assert_eq!(param_a["range"]["start"]["character"], 11);
    assert_eq!(param_a["range"]["end"]["character"], 12);

    let param_b = items.iter().find(|s| s["name"] == "b").unwrap();
    assert_eq!(param_b["kind"], 13); // Variable / Local parameter
    assert_eq!(param_b["detail"], "b: i32");
    assert_eq!(param_b["range"]["start"]["line"], 0);
    assert_eq!(param_b["range"]["start"]["character"], 19);
    assert_eq!(param_b["range"]["end"]["character"], 20);

    let cfg_sym = items.iter().find(|s| s["name"] == "Config").unwrap();
    assert_eq!(cfg_sym["kind"], 23); // Struct
    assert_eq!(cfg_sym["detail"], Value::Null);
    assert_eq!(cfg_sym["range"]["start"]["line"], 1);
    assert_eq!(cfg_sym["range"]["start"]["character"], 11);
    assert_eq!(cfg_sym["range"]["end"]["character"], 17);
}

#[test]
fn test_l1_document_symbol_flat_list_no_children() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/flat.rs";
    let file = "/workspace/src/flat.rs";
    let code = "pub struct Outer {\n    pub field: i32,\n}\nimpl Outer {\n    pub fn method(&self) {}\n}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol array");
    assert_eq!(items.len(), 2);

    // Current RIDX specification invariant: response is flat list, no children array
    for item in items {
        assert!(item.get("children").is_none(), "RIDX document symbols must remain flat");
    }

    let names: Vec<&str> = items.iter().filter_map(|s| s["name"].as_str()).collect();
    assert_eq!(names, vec!["Outer", "method"]);

    // Both struct and impl method appear at top level with exact source lines
    assert_eq!(items[0]["name"], "Outer");
    assert_eq!(items[0]["kind"], 23); // Struct
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[0]["range"]["start"]["character"], 11);
    assert_eq!(items[0]["range"]["end"]["character"], 16);

    assert_eq!(items[1]["name"], "method");
    assert_eq!(items[1]["kind"], 12); // Function in overlay parser
    assert_eq!(items[1]["range"]["start"]["line"], 4);
    assert_eq!(items[1]["range"]["start"]["character"], 11);
    assert_eq!(items[1]["range"]["end"]["character"], 17);
}

#[test]
fn test_l1_document_symbol_enum_structure() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/types.rs";
    let file = "/workspace/src/types.rs";
    let code = "pub enum Color {\n    Red,\n    Green,\n    Blue,\n}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol array");

    // Enum item is discovered with kind 10 (Enum); variants are not top-level items in overlay parser
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Color");
    assert_eq!(items[0]["kind"], 10);
    assert_eq!(items[0]["range"]["start"]["line"], 0);
    assert_eq!(items[0]["range"]["start"]["character"], 9);
    assert_eq!(items[0]["range"]["end"]["character"], 14);
}

#[test]
fn test_l1_document_symbol_mod_blocks() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/network.rs";
    let file = "/workspace/src/network.rs";
    let code = "pub mod telemetry {\n    pub fn track() {}\n}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol array");

    assert_eq!(items.len(), 2);
    let mod_sym = items.iter().find(|s| s["name"] == "telemetry").unwrap();
    assert_eq!(mod_sym["kind"], 2); // Module
    assert_eq!(mod_sym["range"]["start"]["line"], 0);
    assert_eq!(mod_sym["range"]["start"]["character"], 8);
    assert_eq!(mod_sym["range"]["end"]["character"], 17);

    let fn_sym = items.iter().find(|s| s["name"] == "track").unwrap();
    assert_eq!(fn_sym["kind"], 12); // Function
    assert_eq!(fn_sym["range"]["start"]["line"], 1);
    assert_eq!(fn_sym["range"]["start"]["character"], 11);
    assert_eq!(fn_sym["range"]["end"]["character"], 16);
}

#[test]
fn test_l1_document_symbol_const_static_type_alias() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/defs.rs";
    let file = "/workspace/src/defs.rs";
    let code = "pub const CACHE_SIZE: usize = 1024;\npub static RUNTIME_FLAG: bool = true;\npub type Handler = fn(i32);\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol array");
    assert_eq!(items.len(), 3);

    let const_sym = items.iter().find(|s| s["name"] == "CACHE_SIZE").unwrap();
    assert_eq!(const_sym["kind"], 14); // Constant
    assert_eq!(const_sym["range"]["start"]["line"], 0);
    assert_eq!(const_sym["range"]["start"]["character"], 10);
    assert_eq!(const_sym["range"]["end"]["character"], 20);

    let static_sym = items.iter().find(|s| s["name"] == "RUNTIME_FLAG").unwrap();
    assert_eq!(static_sym["kind"], 13); // Variable / Static
    assert_eq!(static_sym["range"]["start"]["line"], 1);
    assert_eq!(static_sym["range"]["start"]["character"], 11);
    assert_eq!(static_sym["range"]["end"]["character"], 23);

    let type_sym = items.iter().find(|s| s["name"] == "Handler").unwrap();
    assert_eq!(type_sym["kind"], 26); // TypeParameter / TyAlias
    assert_eq!(type_sym["range"]["start"]["line"], 2);
    assert_eq!(type_sym["range"]["start"]["character"], 9);
    assert_eq!(type_sym["range"]["end"]["character"], 16);
}

#[test]
fn test_l1_document_symbol_trait_with_methods() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/traits.rs";
    let file = "/workspace/src/traits.rs";
    let code = "pub trait Storage {\n    fn save(&self) -> bool;\n    fn load(&self) -> i32 { 0 }\n}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = make_doc_req(uri);
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol array");
    assert_eq!(items.len(), 3);

    let trait_sym = items.iter().find(|s| s["name"] == "Storage").unwrap();
    assert_eq!(trait_sym["kind"], 11); // Interface / Trait
    assert_eq!(trait_sym["range"]["start"]["line"], 0);
    assert_eq!(trait_sym["range"]["start"]["character"], 10);
    assert_eq!(trait_sym["range"]["end"]["character"], 17);

    let save_fn = items.iter().find(|s| s["name"] == "save").unwrap();
    assert_eq!(save_fn["kind"], 12); // Function
    assert_eq!(save_fn["detail"], "fn save(&self) -> bool");
    assert_eq!(save_fn["range"]["start"]["line"], 1);
    assert_eq!(save_fn["range"]["start"]["character"], 7);
    assert_eq!(save_fn["range"]["end"]["character"], 11);

    let load_fn = items.iter().find(|s| s["name"] == "load").unwrap();
    assert_eq!(load_fn["kind"], 12); // Function
    assert_eq!(load_fn["detail"], "fn load(&self) -> i32");
    assert_eq!(load_fn["range"]["start"]["line"], 2);
    assert_eq!(load_fn["range"]["start"]["character"], 7);
    assert_eq!(load_fn["range"]["end"]["character"], 11);
}

#[test]
fn test_l1_document_symbol_multi_document_isolation() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri_a = "file:///workspace/src/alpha.rs";
    let file_a = "/workspace/src/alpha.rs";
    let uri_b = "file:///workspace/src/beta.rs";
    let file_b = "/workspace/src/beta.rs";

    overlay_mgr.open_document(uri_a, file_a, 1, "pub fn alpha_task() {}\n".to_string());
    overlay_mgr.open_document(uri_b, file_b, 1, "pub fn beta_worker() {}\n".to_string());

    let req_a = make_doc_req(uri_a);
    let res_a = handle_document_symbol(&req_a, &overlay_mgr, &mut conv);
    let items_a = res_a.as_array().expect("array a");
    assert_eq!(items_a.len(), 1);
    assert_eq!(items_a[0]["name"], "alpha_task");

    let req_b = make_doc_req(uri_b);
    let res_b = handle_document_symbol(&req_b, &overlay_mgr, &mut conv);
    let items_b = res_b.as_array().expect("array b");
    assert_eq!(items_b.len(), 1);
    assert_eq!(items_b[0]["name"], "beta_worker");

    let req_unopened = make_doc_req("file:///workspace/src/unopened.rs");
    let res_unopened = handle_document_symbol(&req_unopened, &overlay_mgr, &mut conv);
    assert_eq!(res_unopened, json!([]));
}

#[test]
fn test_l1_document_symbol_did_change_dynamic_refresh() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/live.rs";
    let file = "/workspace/src/live.rs";

    // Version 1: initial function
    overlay_mgr.open_document(uri, file, 1, "pub fn initial_func() {}\n".to_string());
    let req = make_doc_req(uri);
    let res1 = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items1 = res1.as_array().expect("array v1");
    assert_eq!(items1.len(), 1);
    assert_eq!(items1[0]["name"], "initial_func");

    // Version 2: append another function
    overlay_mgr.update(file, 2, "pub fn initial_func() {}\npub fn appended_func() {}\n".to_string());
    let res2 = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items2 = res2.as_array().expect("array v2");
    assert_eq!(items2.len(), 2);
    let names2: Vec<&str> = items2.iter().filter_map(|s| s["name"].as_str()).collect();
    assert_eq!(names2, vec!["initial_func", "appended_func"]);

    // Version 3: replace entire contents with a struct
    overlay_mgr.update(file, 3, "pub struct ReplacedModel;\n".to_string());
    let res3 = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items3 = res3.as_array().expect("array v3");
    assert_eq!(items3.len(), 1);
    assert_eq!(items3[0]["name"], "ReplacedModel");
    assert_eq!(items3[0]["kind"], 23);
}
