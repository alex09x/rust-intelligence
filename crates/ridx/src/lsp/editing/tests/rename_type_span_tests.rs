/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use crate::lsp::editing::rename::handle_rename;
use crate::lsp::protocol::{path_to_uri, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference, WorkspaceIndex};
use serde_json::json;
use std::fs;

fn reference(file: &str, src: &str, cols: (u32, u32), line: u32) -> RawReference {
    span_reference(file, src, (line, cols.0, line, cols.1))
}

/// A reference over `(start line, start col, end line, end col)`, 1-based.
fn span_reference(file: &str, src: &str, span: (u32, u32, u32, u32)) -> RawReference {
    RawReference {
        file: file.to_string(),
        start_line: span.0,
        start_col: span.1,
        end_line: span.2,
        end_col: span.3,
        target_def_id: Some(1),
        target_path: "test_crate::foo".to_string(),
        target_unit: None,
        target_file: Some(src.to_string()),
        target_line: 1,
        target_col: 8,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    }
}

/// A reference over an attribute that runs over several lines starts on `#`, not on a name; it
/// covers identifiers and is skipped like the one-line type span below.
#[test]
fn test_rename_skips_a_reference_spanning_several_lines() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_rename_multi_span_{}", std::process::id()));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let caller_file = temp_dir.join("src/caller.rs");
    fs::write(&src_file, "pub fn foo() {}\n").unwrap();
    fs::write(&caller_file, "fn c() { foo(); }\n#[serde(skip,\n  default)]\nx: u8,\n").unwrap();
    let src_str = src_file.to_str().unwrap().to_string();
    let caller_str = caller_file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("test_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: src_str.clone(),
        name: "foo".to_string(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 11,
        path: "test_crate::foo".to_string(),
        signature: Some("fn()".to_string()),
    });
    writer.add_reference(reference(&caller_str, &src_str, (10, 13), 1));
    writer.add_reference(span_reference(&caller_str, &src_str, (2, 1, 3, 11)));
    let ridx_file = temp_dir.join("index.ridx");
    writer.write_to(&mut fs::File::create(&ridx_file).unwrap()).unwrap();

    let ws = WorkspaceIndex::open(&ridx_file).unwrap();
    let req = json!({"params": {
        "textDocument": { "uri": path_to_uri(&src_str) },
        "position": { "line": 0, "character": 7 },
        "newName": "bar"
    }});
    let res = handle_rename(&req, Some(&ws), &OverlayManager::new(), &mut PositionConverter::new()).expect("rename should succeed");
    let edits = res["changes"][path_to_uri(&caller_str)].as_array().unwrap().clone();
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0]["range"]["start"]["line"], 0);
    let _ = fs::remove_dir_all(&temp_dir);
}

/// A reference a derive wrote over a field type (`&'static str`) is recorded at the span of that
/// type, on no identifier of the renamed symbol: it is skipped, the rename of the real
/// occurrences goes through. A blank position stays an error (`rename_macro_tests`).
#[test]
fn test_rename_skips_a_reference_spanning_a_type_expression() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_rename_type_span_{}", std::process::id()));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let caller_file = temp_dir.join("src/caller.rs");
    fs::write(&src_file, "pub fn foo() {}\n").unwrap();
    fs::write(&caller_file, "fn c() { foo(); }\nVar(&'static str),\n").unwrap();
    let src_str = src_file.to_str().unwrap().to_string();
    let caller_str = caller_file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("test_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: src_str.clone(),
        name: "foo".to_string(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 11,
        path: "test_crate::foo".to_string(),
        signature: Some("fn()".to_string()),
    });
    writer.add_reference(reference(&caller_str, &src_str, (10, 13), 1));
    writer.add_reference(reference(&caller_str, &src_str, (5, 17), 2));
    let ridx_file = temp_dir.join("index.ridx");
    writer.write_to(&mut fs::File::create(&ridx_file).unwrap()).unwrap();

    let ws = WorkspaceIndex::open(&ridx_file).unwrap();
    let req = json!({"params": {
        "textDocument": { "uri": path_to_uri(&src_str) },
        "position": { "line": 0, "character": 7 },
        "newName": "bar"
    }});
    let res = handle_rename(&req, Some(&ws), &OverlayManager::new(), &mut PositionConverter::new()).expect("rename should succeed");
    let edits = res["changes"][path_to_uri(&caller_str)].as_array().unwrap().clone();
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0]["range"]["start"]["line"], 0);
    assert_eq!(edits[0]["range"]["start"]["character"], 9);
    let _ = fs::remove_dir_all(&temp_dir);
}
