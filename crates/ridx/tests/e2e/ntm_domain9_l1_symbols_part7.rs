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

fn sym_item(
    name: &str,
    kind: u32,
    line: u32,
    start_char: u32,
    end_char: u32,
    detail: Option<&str>,
) -> serde_json::Value {
    let range = json!({
        "start": { "line": line, "character": start_char },
        "end": { "line": line, "character": end_char }
    });
    json!({
        "name": name,
        "kind": kind,
        "range": range,
        "selectionRange": range,
        "detail": detail
    })
}

#[test]
fn test_document_symbol_unopened_file_matches_opened_file_exact_json() {
    use ridx::lsp::dispatch::dispatch_lsp_message;
    use ridx::lsp::lifecycle::LifecycleManager;
    use ridx::lsp::protocol::path_to_uri;
    use ridx::lsp::workspace::MultiRootWorkspace;
    use serde_json::Value;
    use std::collections::HashSet;
    use std::io::Cursor;
    use std::sync::{Arc, Mutex};

    let temp_dir =
        std::env::temp_dir().join(format!("ridx_doc_sym_unopened_eq_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let src_file = temp_dir.join("lib.rs");
    let file_str = src_file.to_str().unwrap().to_string();
    let code = concat!(
        "// Emoji header 🦀\n",
        "pub struct Config {\n    pub retries: u32,\n}\n",
        "pub enum Mode {\n    Fast,\n    Safe,\n}\n",
        "pub trait Runner {\n    fn execute(&self) -> i32;\n}\n",
        "pub const DEFAULT_LIMIT: usize = 16;\n",
        "pub fn identity<T>(val: T) -> T {\n    val\n}\n",
    );
    std::fs::write(&src_file, code).unwrap();

    let uri = path_to_uri(&file_str);
    let mut lifecycle = LifecycleManager::new();
    let _ = lifecycle.handle_initialize(&json!({ "params": {} }));
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let mut out_closed = Cursor::new(Vec::new());
    let _ = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "textDocument/documentSymbol",
            "params": { "textDocument": { "uri": &uri } }
        }),
        &mut out_closed,
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        None,
    );
    let raw_closed = String::from_utf8(out_closed.into_inner()).unwrap();
    let resp_closed: Value =
        serde_json::from_str(raw_closed.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    let symbols_closed = resp_closed["result"].clone();

    let mut out_open = Cursor::new(Vec::new());
    let _ = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": { "uri": &uri, "version": 1, "text": code }
            }
        }),
        &mut out_open,
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        None,
    );

    let mut out_opened = Cursor::new(Vec::new());
    let _ = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "textDocument/documentSymbol",
            "params": { "textDocument": { "uri": &uri } }
        }),
        &mut out_opened,
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        None,
    );
    let raw_opened = String::from_utf8(out_opened.into_inner()).unwrap();
    let resp_opened: Value =
        serde_json::from_str(raw_opened.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    let symbols_opened = resp_opened["result"].clone();
    let _ = std::fs::remove_dir_all(&temp_dir);

    let opened_arr = symbols_opened.as_array().expect("opened symbols array");
    assert_eq!(opened_arr.len(), 7);
    assert_eq!(
        symbols_closed, symbols_opened,
        "documentSymbol without didOpen must equal documentSymbol after didOpen with identical text"
    );
}

#[test]
fn test_document_symbol_closed_to_open_modified_to_closed_disk_fallback() {
    use ridx::lsp::protocol::path_to_uri;

    let temp_dir =
        std::env::temp_dir().join(format!("ridx_doc_sym_lifecycle_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let src_file = temp_dir.join("mod_item.rs");
    let file_str = src_file.to_str().unwrap().to_string();
    std::fs::write(&src_file, "pub fn disk_only() -> u32 {\n    7\n}\n").unwrap();

    let uri = path_to_uri(&file_str);
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let req = json!({ "params": { "textDocument": { "uri": &uri } } });

    let res_before_open = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    assert_eq!(
        res_before_open,
        json!([sym_item(
            "disk_only",
            12,
            0,
            7,
            16,
            Some("fn disk_only() -> u32")
        )])
    );

    overlay_mgr.open_document(
        &uri,
        &file_str,
        1,
        "pub struct LiveBufferItem;\npub fn live_fn() {}\n".to_string(),
    );
    let res_while_open = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    assert_eq!(
        res_while_open,
        json!([
            sym_item("LiveBufferItem", 23, 0, 11, 25, None),
            sym_item("live_fn", 12, 1, 7, 14, Some("fn live_fn()"))
        ])
    );

    overlay_mgr.close_document(&file_str);
    let res_after_close = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let _ = std::fs::remove_dir_all(&temp_dir);

    assert_eq!(res_after_close, res_before_open);
}

#[test]
fn test_document_symbol_disk_cache_invalidates_after_disk_write_and_delete() {
    use ridx::lsp::protocol::path_to_uri;

    let temp_dir =
        std::env::temp_dir().join(format!("ridx_doc_sym_cache_inval_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let src_file = temp_dir.join("cached.rs");
    let file_str = src_file.to_str().unwrap().to_string();
    std::fs::write(&src_file, "pub fn first_disk_fn() {}\n").unwrap();

    let uri = path_to_uri(&file_str);
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let req = json!({ "params": { "textDocument": { "uri": &uri } } });

    let res_v1_cold = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let res_v1_warm = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let expected_v1 = json!([sym_item(
        "first_disk_fn",
        12,
        0,
        7,
        20,
        Some("fn first_disk_fn()")
    )]);
    assert_eq!(res_v1_cold, expected_v1);
    assert_eq!(res_v1_warm, expected_v1);

    std::fs::write(
        &src_file,
        "pub fn first_disk_fn() {}\npub struct AddedOnDisk;\n",
    )
    .unwrap();
    let res_v2 = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    assert_eq!(
        res_v2,
        json!([
            sym_item("first_disk_fn", 12, 0, 7, 20, Some("fn first_disk_fn()")),
            sym_item("AddedOnDisk", 23, 1, 11, 22, None)
        ])
    );

    std::fs::remove_file(&src_file).unwrap();
    let res_deleted = handle_document_symbol(&req, &overlay_mgr, &mut conv);
    let dir_uri = path_to_uri(temp_dir.to_str().unwrap());
    let res_dir = handle_document_symbol(
        &json!({ "params": { "textDocument": { "uri": dir_uri } } }),
        &overlay_mgr,
        &mut conv,
    );
    let _ = std::fs::remove_dir_all(&temp_dir);

    assert_eq!(res_deleted, json!([]));
    assert_eq!(res_dir, json!([]));
}
