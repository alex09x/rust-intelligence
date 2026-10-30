/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unit tests verifying fallback behavior for sysroot types (P2.2) and unindexed types.

use crate::lsp::intelligence::handle_completion;
use crate::lsp::protocol::PositionConverter;
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawType, WorkspaceIndex};
use serde_json::json;
use std::fs;

#[test]
fn test_completion_sysroot_fallback_dot() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_sysroot_dot_{}", std::process::id()));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let code = "fn main() {\n    let s = \"hello\";\n    s.l\n}\n";
    let _ = fs::write(&src_file, code);

    let src_path = src_file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("test_sysroot", "0.1.0");

    // Add a symbol that matches fallback search
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::AssocFn,
        visibility: 0,
        is_macro_expansion: false,
        file: src_path.clone(),
        name: "len".to_string(),
        start_line: 10,
        start_col: 5,
        end_line: 10,
        end_col: 8,
        path: "test_sysroot::str::len".to_string(),
        signature: Some("fn len(&self) -> usize".to_string()),
    });

    // Expression `s` has owner "str" (sysroot type)
    writer.add_type_with_owner(RawType { file: src_path.clone(), start_line: 3, start_col: 5, end_line: 3, end_col: 6, type_name: "&str".to_string() }, Some("str".to_string()));

    let ridx_file = temp_dir.join("index.ridx");
    let mut f = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Query completion at `s.l` (line 3, col 7, 0-based: line 2, character 7)
    let uri = format!("file://{}", src_file.to_str().unwrap());
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 7 }
        }
    });

    // Owner completion skips "str" because it is a sysroot path (P2.2),
    // and cleanly falls back to workspace search without panicking.
    let res = handle_completion(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(res["items"].is_array());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_completion_unknown_type_fallback_dot() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_unknown_dot_{}", std::process::id()));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let code = "fn main() {\n    let x = 123;\n    x.fo\n}\n";
    let _ = fs::write(&src_file, code);

    let src_path = src_file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("test_unknown", "0.1.0");

    // Add a definition with no owner index match
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: src_path.clone(),
        name: "foo".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 4,
        path: "test_unknown::foo".to_string(),
        signature: Some("fn foo()".to_string()),
    });

    let ridx_file = temp_dir.join("index.ridx");
    let mut f = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Query completion at `x.fo` (line 3, col 8, 0-based: line 2, character 8)
    let uri = format!("file://{}", src_file.to_str().unwrap());
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 8 }
        }
    });

    let res = handle_completion(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(res["items"].is_array());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_completion_self_colon_colon() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_self_colon_{}", std::process::id()));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let code = "pub struct MyCounter;\nimpl MyCounter {\n    pub fn increment() {}\n    pub fn test() {\n        Self::\n    }\n}\n";
    let _ = fs::write(&src_file, code);

    let src_path = src_file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("test_self", "0.1.0");

    // Definition of MyCounter struct
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 1,
        is_macro_expansion: false,
        file: src_path.clone(),
        name: "MyCounter".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 22,
        path: "test_self::MyCounter".to_string(),
        signature: Some("pub struct MyCounter;".to_string()),
    });

    // Associated function increment with owner test_self::MyCounter
    writer.add_definition_with_owners(
        RawDefinition {
            def_id: 2,
            kind: DefKind::AssocFn,
            visibility: 1,
            is_macro_expansion: false,
            file: src_path.clone(),
            name: "increment".to_string(),
            start_line: 3,
            start_col: 5,
            end_line: 3,
            end_col: 26,
            path: "test_self::MyCounter::increment".to_string(),
            signature: Some("pub fn increment()".to_string()),
        },
        vec!["test_self::MyCounter".to_string()],
    );

    let ridx_file = temp_dir.join("index.ridx");
    let mut f = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Query completion at `Self::` (line 5, character 14, 0-based: line 4, character 14)
    let uri = format!("file://{}", src_file.to_str().unwrap());
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 4, "character": 14 }
        }
    });

    let res = handle_completion(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(res["items"].is_array());
    let items = res["items"].as_array().unwrap();
    assert!(!items.is_empty(), "Items should not be empty for Self::");
    let inc_item = items.iter().find(|it| it["label"] == "increment");
    assert!(inc_item.is_some(), "increment method should be found via Self::");
    let detail = inc_item.unwrap()["detail"].as_str().unwrap();
    assert!(detail.contains("test_self::MyCounter"), "detail should link to owner path: {detail}");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_completion_self_colon_colon_type_position_filters_methods() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_self_colon_type_{}", std::process::id()));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let code =
        "pub struct UniqueEntityIter;\nimpl Iterator for UniqueEntityIter {\n    type Item = u32;\n    fn next(&mut self) -> Option<Self::Item> { None }\n    fn reduce<F>(self, f: F) -> Option<Self::> { None }\n}\n";
    let _ = fs::write(&src_file, code);

    let src_path = src_file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("test_iter", "0.1.0");

    // Definition of UniqueEntityIter struct
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 1,
        is_macro_expansion: false,
        file: src_path.clone(),
        name: "UniqueEntityIter".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 28,
        path: "test_iter::UniqueEntityIter".to_string(),
        signature: Some("pub struct UniqueEntityIter;".to_string()),
    });

    // Associated type Item on UniqueEntityIter
    writer.add_definition_with_owners(
        RawDefinition {
            def_id: 2,
            kind: DefKind::AssocTy,
            visibility: 1,
            is_macro_expansion: false,
            file: src_path.clone(),
            name: "Item".to_string(),
            start_line: 3,
            start_col: 5,
            end_line: 3,
            end_col: 21,
            path: "test_iter::UniqueEntityIter::Item".to_string(),
            signature: Some("type Item = u32".to_string()),
        },
        vec!["test_iter::UniqueEntityIter".to_string()],
    );

    // Associated function next on UniqueEntityIter
    writer.add_definition_with_owners(
        RawDefinition {
            def_id: 3,
            kind: DefKind::AssocFn,
            visibility: 1,
            is_macro_expansion: false,
            file: src_path.clone(),
            name: "next".to_string(),
            start_line: 4,
            start_col: 5,
            end_line: 4,
            end_col: 54,
            path: "test_iter::UniqueEntityIter::next".to_string(),
            signature: Some("fn next(&mut self) -> Option<Self::Item>".to_string()),
        },
        vec!["test_iter::UniqueEntityIter".to_string()],
    );

    // Associated function reduce on UniqueEntityIter
    writer.add_definition_with_owners(
        RawDefinition {
            def_id: 4,
            kind: DefKind::AssocFn,
            visibility: 1,
            is_macro_expansion: false,
            file: src_path.clone(),
            name: "reduce".to_string(),
            start_line: 5,
            start_col: 5,
            end_line: 5,
            end_col: 56,
            path: "test_iter::UniqueEntityIter::reduce".to_string(),
            signature: Some("fn reduce<F>(self, f: F) -> Option<Self::Item>".to_string()),
        },
        vec!["test_iter::UniqueEntityIter".to_string()],
    );

    let ridx_file = temp_dir.join("index.ridx");
    let mut f = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Line 5 (0-based line 4): `    fn reduce<F>(self, f: F) -> Option<Self::> { None }`
    // `Option<Self::` is 45 characters long (0-based index 45 points right after `::`)
    let uri = format!("file://{}", src_file.to_str().unwrap());
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 4, "character": 45 }
        }
    });

    let res = handle_completion(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(res["items"].is_array());
    let items = res["items"].as_array().unwrap();

    let labels: Vec<&str> = items.iter().filter_map(|it| it["label"].as_str()).collect();
    assert_eq!(labels, vec!["Item"], "In Option<Self::|>, only Item should be offered, but got: {:?}", labels);

    let _ = fs::remove_dir_all(&temp_dir);
}
