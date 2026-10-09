/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unit tests for textDocument/prepareRename.
//!
//! Validates placeholder, range, boundary conditions, external symbols, and rejection.

use crate::lsp::editing::rename::handle_prepare_rename;
use crate::lsp::protocol::{path_to_uri, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::DefKind;
use ridx_test_support::{parse_fixture, FixtureIndexBuilder};
use serde_json::{json, Value};
use std::fs;

fn prepare_rename_at(
    uri: &str,
    line: u32,
    character: u32,
    ws: Option<&ridx_format::WorkspaceIndex>,
    overlay_mgr: &OverlayManager,
    conv: &mut PositionConverter,
) -> Value {
    let req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character }
        }
    });
    handle_prepare_rename(&req, ws, overlay_mgr, conv)
}

#[test]
fn test_prepare_rename_local_binding_declaration() {
    let fixture = "fn compute() {\n    let val$0ue = 42;\n    value + 1\n}\n";
    let parsed = parse_fixture(fixture);
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("lib.rs");
    let file_str = file.to_str().unwrap();
    let uri = path_to_uri(file_str);

    let mut overlay_mgr = OverlayManager::new();
    overlay_mgr.open_document(&uri, file_str, 1, parsed.clean_text.clone());
    let mut conv = PositionConverter::new();

    let cur = parsed.cursor.unwrap();
    let res = prepare_rename_at(&uri, cur.line, cur.col_utf16, None, &overlay_mgr, &mut conv);
    assert!(!res.is_null());
    assert_eq!(res["placeholder"], "value");
    assert_eq!(res["range"]["start"]["line"], 1);
    assert_eq!(res["range"]["start"]["character"], 8);
    assert_eq!(res["range"]["end"]["character"], 13);
}

#[test]
fn test_prepare_rename_local_binding_reference() {
    let fixture = "fn compute() {\n    let value = 42;\n    val$0ue + 1\n}\n";
    let parsed = parse_fixture(fixture);
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("lib.rs");
    let file_str = file.to_str().unwrap();
    let uri = path_to_uri(file_str);

    let mut overlay_mgr = OverlayManager::new();
    overlay_mgr.open_document(&uri, file_str, 1, parsed.clean_text);
    let mut conv = PositionConverter::new();

    let cur = parsed.cursor.unwrap();
    let res = prepare_rename_at(&uri, cur.line, cur.col_utf16, None, &overlay_mgr, &mut conv);
    assert!(!res.is_null());
    assert_eq!(res["placeholder"], "value");
    assert_eq!(res["range"]["start"]["line"], 2);
    assert_eq!(res["range"]["start"]["character"], 4);
    assert_eq!(res["range"]["end"]["character"], 9);
}

#[test]
fn test_prepare_rename_at_identifier_boundary() {
    // Cursor placed right at the boundary after 'value': `value$0 = 42`
    let fixture = "fn compute() {\n    let value$0 = 42;\n    value + 1\n}\n";
    let parsed = parse_fixture(fixture);
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("lib.rs");
    let file_str = file.to_str().unwrap();
    let uri = path_to_uri(file_str);

    let mut overlay_mgr = OverlayManager::new();
    overlay_mgr.open_document(&uri, file_str, 1, parsed.clean_text);
    let mut conv = PositionConverter::new();

    let cur = parsed.cursor.unwrap();
    let res = prepare_rename_at(&uri, cur.line, cur.col_utf16, None, &overlay_mgr, &mut conv);
    assert!(!res.is_null());
    assert_eq!(res["placeholder"], "value");
    assert_eq!(res["range"]["start"]["character"], 8);
    assert_eq!(res["range"]["end"]["character"], 13);
}

#[test]
fn test_prepare_rename_fn_parameter() {
    let fixture = "fn add(x$0: i32, y: i32) -> i32 { x + y }\n";
    let parsed = parse_fixture(fixture);
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("lib.rs");
    let file_str = file.to_str().unwrap();
    let uri = path_to_uri(file_str);

    let mut overlay_mgr = OverlayManager::new();
    overlay_mgr.open_document(&uri, file_str, 1, parsed.clean_text);
    let mut conv = PositionConverter::new();

    let cur = parsed.cursor.unwrap();
    let res = prepare_rename_at(&uri, cur.line, cur.col_utf16, None, &overlay_mgr, &mut conv);
    assert!(!res.is_null());
    assert_eq!(res["placeholder"], "x");
}

#[test]
fn test_prepare_rename_compiled_symbol() {
    let temp = tempfile::tempdir().unwrap();
    let src_file = temp.path().join("lib.rs");
    let src_str = src_file.to_str().unwrap().to_string();
    let uri = path_to_uri(&src_str);

    let fixture = "pub fn «$0calculate»(x: i32) -> i32 { x * 2 }\n";
    let mut builder = FixtureIndexBuilder::new("my_crate", "0.1.0");
    let (parsed, _def_ids) =
        builder.add_defs_and_self_refs_fixture(fixture, &src_str, "my_crate", DefKind::Fn);
    fs::write(&src_file, &parsed.clean_text).unwrap();
    let idx_file = temp.path().join("index.ridx");
    builder.write_to_file(&idx_file);
    let ws = ridx_format::WorkspaceIndex::open(&idx_file).unwrap();

    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let cur = parsed.cursor.unwrap();
    let res = prepare_rename_at(
        &uri,
        cur.line,
        cur.col_utf16,
        Some(&ws),
        &overlay_mgr,
        &mut conv,
    );
    assert!(!res.is_null());
    assert_eq!(res["placeholder"], "calculate");
    assert_eq!(res["range"]["start"]["character"], 7); // 0-based col
    assert_eq!(res["range"]["end"]["character"], 16);
}

#[test]
fn test_prepare_rename_rejects_external_symbol() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("app");
    let src_file = app_dir.join("src/lib.rs");
    fs::create_dir_all(src_file.parent().unwrap()).unwrap();
    let src_str = src_file.to_str().unwrap().to_string();
    let uri = path_to_uri(&src_str);

    // Realistic external dependency file outside workspace root (no magic substring)
    let ext_dep = temp.path().join("elsewhere/serde-1.0.0/src/lib.rs");
    fs::create_dir_all(ext_dep.parent().unwrap()).unwrap();
    fs::write(&ext_dep, "pub trait Serialize {}\n").unwrap();
    let ext_dep_str = ext_dep.to_str().unwrap();

    let mut builder = FixtureIndexBuilder::new("app", "0.1.0");
    let fixture = "use serde::«$0Serialize»;\n";
    let parsed = parse_fixture(fixture);
    fs::write(&src_file, &parsed.clean_text).unwrap();
    builder.add_refs_from_fixture(
        fixture,
        &src_str,
        "serde::Serialize",
        None,
        Some(ext_dep_str),
        1,
        1,
        DefKind::Trait,
    );
    let idx_file = app_dir.join("target/ridx/app.ridx");
    fs::create_dir_all(idx_file.parent().unwrap()).unwrap();
    builder.write_to_file(&idx_file);
    let mut ws = ridx_format::WorkspaceIndex::open(&idx_file).unwrap();
    ws.set_workspace_root(Some(app_dir));

    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let cur = parsed.cursor.unwrap();
    let res = prepare_rename_at(
        &uri,
        cur.line,
        cur.col_utf16,
        Some(&ws),
        &overlay_mgr,
        &mut conv,
    );
    assert_eq!(res, Value::Null);
}

#[test]
fn test_prepare_rename_with_syntax_errors() {
    let fixture = "fn broken() {\n    let val$0ue = 42;\n    value +\n";
    let parsed = parse_fixture(fixture);
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("lib.rs");
    let file_str = file.to_str().unwrap();
    let uri = path_to_uri(file_str);

    let mut overlay_mgr = OverlayManager::new();
    overlay_mgr.open_document(&uri, file_str, 1, parsed.clean_text);
    let mut conv = PositionConverter::new();

    let cur = parsed.cursor.unwrap();
    let res = prepare_rename_at(&uri, cur.line, cur.col_utf16, None, &overlay_mgr, &mut conv);
    assert!(!res.is_null());
    assert_eq!(res["placeholder"], "value");
}

#[test]
fn test_prepare_rename_rejects_keywords() {
    let code = "fn compute() { let x = 1; }\n";
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("lib.rs");
    let file_str = file.to_str().unwrap();
    let uri = path_to_uri(file_str);

    let mut overlay_mgr = OverlayManager::new();
    overlay_mgr.open_document(&uri, file_str, 1, code.to_string());
    let mut conv = PositionConverter::new();

    // Cursor on 'fn' keyword (col 0)
    let res_fn = prepare_rename_at(&uri, 0, 0, None, &overlay_mgr, &mut conv);
    assert_eq!(res_fn, Value::Null);

    // Cursor on 'let' keyword (col 15)
    let res_let = prepare_rename_at(&uri, 0, 15, None, &overlay_mgr, &mut conv);
    assert_eq!(res_let, Value::Null);
}

#[test]
fn test_prepare_rename_rejects_comments_and_strings() {
    let code = "// this is a comment\nlet s = \"string literal\";\n";
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("lib.rs");
    let file_str = file.to_str().unwrap();
    let uri = path_to_uri(file_str);

    let mut overlay_mgr = OverlayManager::new();
    overlay_mgr.open_document(&uri, file_str, 1, code.to_string());
    let mut conv = PositionConverter::new();

    // Cursor on comment
    let res_comment = prepare_rename_at(&uri, 0, 5, None, &overlay_mgr, &mut conv);
    assert_eq!(res_comment, Value::Null);

    // Cursor inside string literal
    let res_str = prepare_rename_at(&uri, 1, 12, None, &overlay_mgr, &mut conv);
    assert_eq!(res_str, Value::Null);
}

#[test]
fn test_prepare_rename_rejects_whitespace_and_punctuation() {
    let code = "fn add(x: i32) -> i32 { x };\n";
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("lib.rs");
    let file_str = file.to_str().unwrap();
    let uri = path_to_uri(file_str);

    let mut overlay_mgr = OverlayManager::new();
    overlay_mgr.open_document(&uri, file_str, 1, code.to_string());
    let mut conv = PositionConverter::new();

    // Whitespace between fn and add (col 2)
    assert_eq!(
        prepare_rename_at(&uri, 0, 2, None, &overlay_mgr, &mut conv),
        Value::Null
    );

    // Open brace '{' (col 22)
    assert_eq!(
        prepare_rename_at(&uri, 0, 22, None, &overlay_mgr, &mut conv),
        Value::Null
    );

    // Arrow '->' (col 15)
    assert_eq!(
        prepare_rename_at(&uri, 0, 15, None, &overlay_mgr, &mut conv),
        Value::Null
    );

    // Semicolon ';' (col 27)
    assert_eq!(
        prepare_rename_at(&uri, 0, 27, None, &overlay_mgr, &mut conv),
        Value::Null
    );
}
