/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::*;
use crate::lsp::symbols::handle_document_symbol;
use serde_json::json;

#[test]
fn test_document_symbols_opened_file_finds_all_kinds() {
    let mut h = setup_test_symbols();
    let code = "pub fn my_symbol() {}\npub struct MyStruct;\npub enum MyEnum {}\npub trait MyTrait {}\npub const MY_CONST: usize = 42;\n";
    h.overlay_mgr
        .open_document(&h.uri, h.src_file.to_str().unwrap(), 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": &h.uri }
        }
    });

    let res = handle_document_symbol(&req, &h.overlay_mgr, &mut h.conv);
    let items = res.as_array().expect("expected array of document symbols");
    assert_eq!(items.len(), 5);

    let names: Vec<&str> = items.iter().filter_map(|s| s["name"].as_str()).collect();
    assert!(names.contains(&"my_symbol"));
    assert!(names.contains(&"MyStruct"));
    assert!(names.contains(&"MyEnum"));
    assert!(names.contains(&"MyTrait"));
    assert!(names.contains(&"MY_CONST"));
}

#[test]
fn test_document_symbols_unopened_file_returns_empty() {
    let mut h = setup_test_symbols();
    let req = json!({
        "params": {
            "textDocument": { "uri": "file:///path/to/unopened_file.rs" }
        }
    });

    let res = handle_document_symbol(&req, &h.overlay_mgr, &mut h.conv);
    let items = res.as_array().expect("expected array");
    assert!(items.is_empty());
}

#[test]
fn test_document_symbols_empty_file_returns_empty() {
    let mut h = setup_test_symbols();
    h.overlay_mgr
        .open_document(&h.uri, h.src_file.to_str().unwrap(), 1, String::new());

    let req = json!({
        "params": {
            "textDocument": { "uri": &h.uri }
        }
    });

    let res = handle_document_symbol(&req, &h.overlay_mgr, &mut h.conv);
    let items = res.as_array().expect("expected array");
    assert!(items.is_empty());
}

#[test]
fn test_document_symbols_comments_only_file_returns_empty() {
    let mut h = setup_test_symbols();
    let code = "// Just a comment\n/* Block comment */\n/// Doc comment without symbol\n";
    h.overlay_mgr
        .open_document(&h.uri, h.src_file.to_str().unwrap(), 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": &h.uri }
        }
    });

    let res = handle_document_symbol(&req, &h.overlay_mgr, &mut h.conv);
    let items = res.as_array().expect("expected array");
    assert!(items.is_empty());
}

#[test]
fn test_document_symbols_selection_range_and_range() {
    let mut h = setup_test_symbols();
    let code = "fn compute_score() -> u32 { 100 }\n";
    h.overlay_mgr
        .open_document(&h.uri, h.src_file.to_str().unwrap(), 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": &h.uri }
        }
    });

    let res = handle_document_symbol(&req, &h.overlay_mgr, &mut h.conv);
    let items = res.as_array().unwrap();
    assert_eq!(items.len(), 1);

    let sym = &items[0];
    assert_eq!(sym["name"], "compute_score");
    assert!(sym.get("range").is_some());
    assert_eq!(sym["range"], sym["selectionRange"]);
    assert_eq!(sym["range"]["start"]["line"], 0);
}

#[test]
fn test_document_symbols_detail_signature() {
    let mut h = setup_test_symbols();
    let code = "pub fn add(a: i32, b: i32) -> i32 { a + b }\n";
    h.overlay_mgr
        .open_document(&h.uri, h.src_file.to_str().unwrap(), 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": &h.uri }
        }
    });

    let res = handle_document_symbol(&req, &h.overlay_mgr, &mut h.conv);
    let items = res.as_array().unwrap();
    // Function plus its two parameters
    assert_eq!(items.len(), 3);

    let fn_sym = items
        .iter()
        .find(|s| s["name"] == "add")
        .expect("add function symbol");
    let detail = fn_sym["detail"].as_str();
    assert!(detail.is_some());
    assert!(detail.unwrap().contains("add"));
}

#[test]
fn test_document_symbols_overlay_update_reflection() {
    let mut h = setup_test_symbols();
    let initial_code = "pub fn first_fn() {}\n";
    h.overlay_mgr.open_document(
        &h.uri,
        h.src_file.to_str().unwrap(),
        1,
        initial_code.to_string(),
    );

    let req = json!({
        "params": {
            "textDocument": { "uri": &h.uri }
        }
    });

    let res1 = handle_document_symbol(&req, &h.overlay_mgr, &mut h.conv);
    let items1 = res1.as_array().unwrap();
    assert_eq!(items1.len(), 1);
    assert_eq!(items1[0]["name"], "first_fn");

    // Modify document through overlay
    let updated_code = "pub fn second_fn() {}\npub fn third_fn() {}\n";
    h.overlay_mgr
        .update(h.src_file.to_str().unwrap(), 2, updated_code.to_string());

    let res2 = handle_document_symbol(&req, &h.overlay_mgr, &mut h.conv);
    let items2 = res2.as_array().unwrap();
    assert_eq!(items2.len(), 2);
    assert_eq!(items2[0]["name"], "second_fn");
    assert_eq!(items2[1]["name"], "third_fn");
}

#[test]
fn test_document_symbols_utf16_multibyte_offsets() {
    let mut h = setup_test_symbols();
    // Line with unicode emojis before symbol
    let code = "// 🦀🚀🌟\npub fn with_emoji_prefix() {}\n";
    h.overlay_mgr
        .open_document(&h.uri, h.src_file.to_str().unwrap(), 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": &h.uri }
        }
    });

    let res = handle_document_symbol(&req, &h.overlay_mgr, &mut h.conv);
    let items = res.as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "with_emoji_prefix");
    assert_eq!(items[0]["range"]["start"]["line"], 1);
}

#[test]
fn test_document_symbols_malformed_request_params() {
    let mut h = setup_test_symbols();
    let empty_req = json!({});
    let res = handle_document_symbol(&empty_req, &h.overlay_mgr, &mut h.conv);
    assert_eq!(res, json!([]));

    let no_uri_req = json!({ "params": { "textDocument": {} } });
    let res2 = handle_document_symbol(&no_uri_req, &h.overlay_mgr, &mut h.conv);
    assert_eq!(res2, json!([]));
}

#[test]
fn test_document_symbols_multiple_documents_isolated() {
    let mut h = setup_test_symbols();
    let file_b = h.temp_dir.join("src/other.rs");
    let uri_b = format!("file://{}", file_b.to_str().unwrap());

    h.overlay_mgr.open_document(
        &h.uri,
        h.src_file.to_str().unwrap(),
        1,
        "pub fn doc_a() {}\n".to_string(),
    );
    h.overlay_mgr.open_document(
        &uri_b,
        file_b.to_str().unwrap(),
        1,
        "pub fn doc_b() {}\n".to_string(),
    );

    let req_a = json!({ "params": { "textDocument": { "uri": &h.uri } } });
    let res_a = handle_document_symbol(&req_a, &h.overlay_mgr, &mut h.conv);
    let items_a = res_a.as_array().unwrap();
    assert_eq!(items_a.len(), 1);
    assert_eq!(items_a[0]["name"], "doc_a");

    let req_b = json!({ "params": { "textDocument": { "uri": &uri_b } } });
    let res_b = handle_document_symbol(&req_b, &h.overlay_mgr, &mut h.conv);
    let items_b = res_b.as_array().unwrap();
    assert_eq!(items_b.len(), 1);
    assert_eq!(items_b[0]["name"], "doc_b");
}
