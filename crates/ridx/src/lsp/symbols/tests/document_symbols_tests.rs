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
    h.overlay_mgr.open_document(&h.uri, h.src_file.to_str().unwrap(), 1, code.to_string());

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
    h.overlay_mgr.open_document(&h.uri, h.src_file.to_str().unwrap(), 1, String::new());

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
    h.overlay_mgr.open_document(&h.uri, h.src_file.to_str().unwrap(), 1, code.to_string());

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
    h.overlay_mgr.open_document(&h.uri, h.src_file.to_str().unwrap(), 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": &h.uri }
        }
    });

    let res = handle_document_symbol(&req, &h.overlay_mgr, &mut h.conv);
    let items = res.as_array().unwrap();
    assert_eq!(items.len(), 1);

    // As in rust-analyzer, `range` spans the whole item and `selectionRange` its name.
    let sym = &items[0];
    assert_eq!(sym["name"], "compute_score");
    assert_eq!(sym["range"], json!({ "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 33 } }));
    assert_eq!(sym["selectionRange"], json!({ "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 16 } }));
}

#[test]
fn test_document_symbols_detail_signature() {
    let mut h = setup_test_symbols();
    let code = "pub fn add(a: i32, b: i32) -> i32 { a + b }\n";
    h.overlay_mgr.open_document(&h.uri, h.src_file.to_str().unwrap(), 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": &h.uri }
        }
    });

    let res = handle_document_symbol(&req, &h.overlay_mgr, &mut h.conv);
    let items = res.as_array().unwrap();
    // Parameters are no symbols; the signature is the function's detail, as rust-analyzer shows it.
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "add");
    assert_eq!(items[0]["detail"], "fn(a: i32, b: i32) -> i32");
}

#[test]
fn test_document_symbols_overlay_update_reflection() {
    let mut h = setup_test_symbols();
    let initial_code = "pub fn first_fn() {}\n";
    h.overlay_mgr.open_document(&h.uri, h.src_file.to_str().unwrap(), 1, initial_code.to_string());

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
    h.overlay_mgr.update(h.src_file.to_str().unwrap(), 2, updated_code.to_string());

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
    h.overlay_mgr.open_document(&h.uri, h.src_file.to_str().unwrap(), 1, code.to_string());

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

    h.overlay_mgr.open_document(&h.uri, h.src_file.to_str().unwrap(), 1, "pub fn doc_a() {}\n".to_string());
    h.overlay_mgr.open_document(&uri_b, file_b.to_str().unwrap(), 1, "pub fn doc_b() {}\n".to_string());

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

#[test]
fn test_document_symbols_closed_file_disk_cache_hits_and_invalidates() {
    use crate::lsp::symbols::document_symbol::document_symbol_cache_hits_for;

    let mut h = setup_test_symbols();
    let file_str = h.src_file.to_str().unwrap();
    let disk_code = std::fs::read_to_string(&h.src_file).unwrap();
    let req = json!({ "params": { "textDocument": { "uri": &h.uri } } });

    // 1. First closed-file request parses from disk and populates the cache (hits == 0).
    let res_closed_1 = handle_document_symbol(&req, &h.overlay_mgr, &mut h.conv);
    assert_eq!(res_closed_1.as_array().unwrap().len(), 5, "fn, struct, enum, trait, const; members are children");
    assert_eq!(document_symbol_cache_hits_for(&h.overlay_mgr, file_str), Some(0));

    // 2. Second closed-file request on unchanged disk file hits the (path, mtime, len) cache.
    let res_closed_2 = handle_document_symbol(&req, &h.overlay_mgr, &mut h.conv);
    assert_eq!(res_closed_1, res_closed_2);
    assert_eq!(document_symbol_cache_hits_for(&h.overlay_mgr, file_str), Some(1));

    // 3. Opening the document with identical text produces the exact same JSON array.
    h.overlay_mgr.open_document(&h.uri, file_str, 1, disk_code);
    let res_opened = handle_document_symbol(&req, &h.overlay_mgr, &mut h.conv);
    assert_eq!(res_closed_1, res_opened);

    // 4. Close the document and modify the file on disk -> cache invalidates (hits resets to 0).
    h.overlay_mgr.close_document(file_str);
    std::fs::write(&h.src_file, "pub fn disk_updated_fn(x: usize) -> usize { x }\n").unwrap();
    let res_updated = handle_document_symbol(&req, &h.overlay_mgr, &mut h.conv);
    let updated_items = res_updated.as_array().unwrap();
    assert_eq!(updated_items.len(), 1);
    assert_eq!(updated_items[0]["name"], "disk_updated_fn");
    assert_eq!(document_symbol_cache_hits_for(&h.overlay_mgr, file_str), Some(0));

    // 5. Deleting the file on disk evicts the cache entry and returns [].
    std::fs::remove_file(&h.src_file).unwrap();
    let res_deleted = handle_document_symbol(&req, &h.overlay_mgr, &mut h.conv);
    assert_eq!(res_deleted, json!([]));
    assert_eq!(document_symbol_cache_hits_for(&h.overlay_mgr, file_str), None);
}

#[test]
fn test_document_symbol_cache_isolated_between_overlay_managers() {
    use crate::lsp::symbols::document_symbol::{document_symbol_cache_hits_for, document_symbol_cache_len};

    let h = setup_test_symbols();
    let file_a_str = h.src_file.to_str().unwrap();
    let req_a = json!({ "params": { "textDocument": { "uri": &h.uri } } });

    let file_b = h.temp_dir.join("src/isolated_second.rs");
    std::fs::write(&file_b, "pub struct SecondItem;\n").unwrap();
    let file_b_str = file_b.to_str().unwrap();
    let uri_b = format!("file://{}", file_b_str);
    let req_b = json!({ "params": { "textDocument": { "uri": &uri_b } } });

    let mgr_a = OverlayManager::new();
    let mgr_b = OverlayManager::new();
    let mut conv = PositionConverter::new();

    assert_eq!(document_symbol_cache_len(&mgr_a), 0);
    assert_eq!(document_symbol_cache_len(&mgr_b), 0);
    assert_eq!(document_symbol_cache_hits_for(&mgr_a, file_a_str), None);
    assert_eq!(document_symbol_cache_hits_for(&mgr_b, file_a_str), None);

    // 1. First closed-file request through mgr_a populates mgr_a's cache (len == 1, hits == 0).
    let res_a1 = handle_document_symbol(&req_a, &mgr_a, &mut conv);
    assert_eq!(res_a1.as_array().unwrap().len(), 5);
    assert_eq!(document_symbol_cache_len(&mgr_a), 1);
    assert_eq!(document_symbol_cache_hits_for(&mgr_a, file_a_str), Some(0));

    // 2. First closed-file request for the SAME file through mgr_b must NOT hit mgr_a's cache:
    // both mgr_b and mgr_a must report hits == Some(0), not Some(1).
    let res_b1 = handle_document_symbol(&req_a, &mgr_b, &mut conv);
    assert_eq!(res_b1, res_a1);
    assert_eq!(document_symbol_cache_hits_for(&mgr_b, file_a_str), Some(0));
    assert_eq!(document_symbol_cache_hits_for(&mgr_a, file_a_str), Some(0));
    assert_eq!(document_symbol_cache_len(&mgr_b), 1);

    // 3. Second request through mgr_a increments only mgr_a's hits to 1; mgr_b stays at 0.
    let res_a2 = handle_document_symbol(&req_a, &mgr_a, &mut conv);
    assert_eq!(res_a2, res_a1);
    assert_eq!(document_symbol_cache_hits_for(&mgr_a, file_a_str), Some(1));
    assert_eq!(document_symbol_cache_hits_for(&mgr_b, file_a_str), Some(0));

    // 4. Requesting a second closed file through mgr_b grows only mgr_b's cache to 2 entries.
    let res_b2 = handle_document_symbol(&req_b, &mgr_b, &mut conv);
    assert_eq!(res_b2.as_array().unwrap().len(), 1);
    assert_eq!(res_b2[0]["name"], "SecondItem");
    assert_eq!(document_symbol_cache_len(&mgr_b), 2);
    assert_eq!(document_symbol_cache_hits_for(&mgr_b, file_b_str), Some(0));
    assert_eq!(document_symbol_cache_len(&mgr_a), 1);
    assert_eq!(document_symbol_cache_hits_for(&mgr_a, file_b_str), None);
}
