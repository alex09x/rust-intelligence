/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/documentSymbol part 7:
//! edge cases, malformed params, UTF-16 emojis, CRLF, nested functions, and const fn.

use ridx::lsp::protocol::PositionConverter;
use ridx::lsp::symbols::handle_document_symbol;
use ridx::overlay::OverlayManager;
use serde_json::json;

#[test]
fn test_document_symbol_empty_uri_returns_empty() {
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let req = json!({
        "params": {
            "textDocument": { "uri": "" }
        }
    });
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]));
}

#[test]
fn test_document_symbol_missing_text_document_param() {
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let req = json!({
        "params": {}
    });
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]));
}

#[test]
fn test_document_symbol_missing_params_object() {
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let req = json!({});
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    assert_eq!(res, json!([]));
}

#[test]
fn test_document_symbol_utf16_surrogate_emoji_in_comment_preserves_coords() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/emoji_symbol.rs";
    let file = "/workspace/src/emoji_symbol.rs";
    let code = "// Emoji prefix 🦀🦀\npub struct CrabItem;\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": uri }
        }
    });
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "CrabItem");
    assert_eq!(items[0]["kind"], 23);
    assert_eq!(items[0]["range"]["start"]["line"], 1);
    assert_eq!(items[0]["range"]["start"]["character"], 11);
    assert_eq!(items[0]["range"]["end"]["character"], 19);
}

#[test]
fn test_document_symbol_crlf_line_endings_preserved() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/crlf_symbols.rs";
    let file = "/workspace/src/crlf_symbols.rs";
    let code = "pub struct WindowsFormat;\r\npub fn win_fn() {}\r\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": uri }
        }
    });
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["name"], "WindowsFormat");
    assert_eq!(items[0]["kind"], 23);
    assert_eq!(items[1]["name"], "win_fn");
    assert_eq!(items[1]["kind"], 12);
}

#[test]
fn test_document_symbol_type_parameter_filtered_out() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/generic_symbols.rs";
    let file = "/workspace/src/generic_symbols.rs";
    let code = "pub struct Wrapper<T> { val: T }\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": uri }
        }
    });
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol list");
    // Wrapper struct discovered; generic parameter T is filtered out
    let typaram_t = items.iter().find(|s| s["name"] == "T");
    assert_eq!(typaram_t, None);
}

#[test]
fn test_document_symbol_nested_functions_in_item() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/nested_fns.rs";
    let file = "/workspace/src/nested_fns.rs";
    let code = "fn outer() {\n    fn inner() {}\n}\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": uri }
        }
    });
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 2);

    let outer_sym = items.iter().find(|s| s["name"] == "outer").unwrap();
    assert_eq!(outer_sym["kind"], 12);

    let inner_sym = items.iter().find(|s| s["name"] == "inner").unwrap();
    assert_eq!(inner_sym["kind"], 12);
}

#[test]
fn test_document_symbol_const_fn_declaration() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/const_fn.rs";
    let file = "/workspace/src/const_fn.rs";
    let code = "pub const fn eval_constant() -> i32 { 10 }\n";
    overlay_mgr.open_document(uri, file, 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": uri }
        }
    });
    let res = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let items = res.as_array().expect("symbol list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "eval_constant");
    assert_eq!(items[0]["kind"], 12); // Function
    assert_eq!(items[0]["detail"], "fn eval_constant() -> i32");
}
