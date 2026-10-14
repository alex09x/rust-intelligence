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
use std::sync::atomic::{AtomicU64, Ordering};

static MACRO_RENAME_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_temp_dir() -> std::path::PathBuf {
    let id = MACRO_RENAME_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_dir = std::env::temp_dir()
        .join(format!("ridx_test_rename_macro_{}_{}", std::process::id(), id));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    temp_dir
}

/// (a) Macro expansion reference without identifier at position is skipped;
/// declaration and normal references are successfully renamed with exact WorkspaceEdit.
#[test]
fn test_rename_macro_reference_without_name_is_skipped() {
    let temp_dir = next_temp_dir();
    let src_file = temp_dir.join("src/lib.rs");
    let caller_file = temp_dir.join("src/caller.rs");
    let macro_file = temp_dir.join("src/macros.rs");

    fs::write(&src_file, "pub fn foo() {}\n").unwrap();
    fs::write(&caller_file, "pub fn call() { foo(); }\n").unwrap();
    // In macro_file, position (line 1, col 35) lands on punctuation, not an ident
    fs::write(&macro_file, "macro_rules! invoke { () => { foo() }; }\n").unwrap();

    let src_str = src_file.to_str().unwrap().to_string();
    let caller_str = caller_file.to_str().unwrap().to_string();
    let macro_str = macro_file.to_str().unwrap().to_string();

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

    // Normal direct reference in caller.rs (line 1, col 17 "foo")
    writer.add_reference(RawReference {
        file: caller_str.clone(),
        start_line: 1,
        start_col: 17,
        end_line: 1,
        end_col: 20,
        target_def_id: Some(1),
        target_path: "test_crate::foo".to_string(),
        target_unit: None,
        target_file: Some(src_str.clone()),
        target_line: 1,
        target_col: 8,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });

    // Synthetic macro expansion reference pointing to column 35 where ident is not extractable
    writer.add_reference(RawReference {
        file: macro_str.clone(),
        start_line: 1,
        start_col: 35,
        end_line: 1,
        end_col: 36,
        target_def_id: Some(1),
        target_path: "test_crate::foo".to_string(),
        target_unit: None,
        target_file: Some(src_str.clone()),
        target_line: 1,
        target_col: 8,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: true,
    });

    let ridx_file = temp_dir.join("index.ridx");
    let mut f = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();

    let ws = WorkspaceIndex::open(&ridx_file).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let src_uri = path_to_uri(&src_str);
    let caller_uri = path_to_uri(&caller_str);
    let macro_uri = path_to_uri(&macro_str);

    let req = json!({
        "params": {
            "textDocument": { "uri": &src_uri },
            "position": { "line": 0, "character": 7 },
            "newName": "bar"
        }
    });

    let res = handle_rename(&req, Some(&ws), &overlay_mgr, &mut conv).expect("rename should succeed");
    let changes = res["changes"].as_object().expect("changes object");

    // Must contain edits for lib.rs and caller.rs
    assert!(changes.contains_key(&src_uri), "lib.rs should be edited");
    assert!(changes.contains_key(&caller_uri), "caller.rs should be edited");
    // Must NOT contain any edit for macros.rs
    assert!(!changes.contains_key(&macro_uri), "macros.rs should be skipped");

    // Exact check on edits
    let src_edits = changes[&src_uri].as_array().unwrap();
    assert_eq!(src_edits.len(), 1);
    assert_eq!(src_edits[0]["newText"], "bar");
    assert_eq!(src_edits[0]["range"]["start"]["line"], 0);
    assert_eq!(src_edits[0]["range"]["start"]["character"], 7);
    assert_eq!(src_edits[0]["range"]["end"]["character"], 10);

    let caller_edits = changes[&caller_uri].as_array().unwrap();
    assert_eq!(caller_edits.len(), 1);
    assert_eq!(caller_edits[0]["newText"], "bar");
    assert_eq!(caller_edits[0]["range"]["start"]["line"], 0);
    assert_eq!(caller_edits[0]["range"]["start"]["character"], 16);
    assert_eq!(caller_edits[0]["range"]["end"]["character"], 19);

    let _ = fs::remove_dir_all(&temp_dir);
}

/// (b) Non-macro reference without identifier at position still returns -32803.
#[test]
fn test_rename_non_macro_reference_without_name_fails_32803() {
    let temp_dir = next_temp_dir();
    let src_file = temp_dir.join("src/lib.rs");
    let caller_file = temp_dir.join("src/caller.rs");

    fs::write(&src_file, "pub fn foo() {}\n").unwrap();
    // caller.rs has no ident at line 1 col 15
    fs::write(&caller_file, "pub fn call() { }\n").unwrap();

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

    // Non-macro reference with bad column
    writer.add_reference(RawReference {
        file: caller_str.clone(),
        start_line: 1,
        start_col: 15,
        end_line: 1,
        end_col: 16,
        target_def_id: Some(1),
        target_path: "test_crate::foo".to_string(),
        target_unit: None,
        target_file: Some(src_str.clone()),
        target_line: 1,
        target_col: 8,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false, // NOT macro expansion!
    });

    let ridx_file = temp_dir.join("index.ridx");
    let mut f = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();

    let ws = WorkspaceIndex::open(&ridx_file).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let src_uri = path_to_uri(&src_str);
    let req = json!({
        "params": {
            "textDocument": { "uri": &src_uri },
            "position": { "line": 0, "character": 7 },
            "newName": "bar"
        }
    });

    let err = handle_rename(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap_err();
    assert_eq!(err.code, -32803);
    assert!(err.message.contains("Failed to locate identifier at reference position"));

    let _ = fs::remove_dir_all(&temp_dir);
}

/// (c) Macro reference where identifier is present at position is renamed.
#[test]
fn test_rename_macro_reference_with_name_is_renamed() {
    let temp_dir = next_temp_dir();
    let src_file = temp_dir.join("src/lib.rs");
    let caller_file = temp_dir.join("src/caller.rs");

    fs::write(&src_file, "pub fn foo() {}\n").unwrap();
    // caller.rs invokes macro with foo as an argument
    fs::write(&caller_file, "log_call!(foo);\n").unwrap();

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

    // Reference in macro invocation where "foo" is at col 11 (1-based: "log_call!(foo);")
    writer.add_reference(RawReference {
        file: caller_str.clone(),
        start_line: 1,
        start_col: 11,
        end_line: 1,
        end_col: 14,
        target_def_id: Some(1),
        target_path: "test_crate::foo".to_string(),
        target_unit: None,
        target_file: Some(src_str.clone()),
        target_line: 1,
        target_col: 8,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: true, // Macro expansion flag is true, BUT name IS present!
    });

    let ridx_file = temp_dir.join("index.ridx");
    let mut f = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();

    let ws = WorkspaceIndex::open(&ridx_file).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let src_uri = path_to_uri(&src_str);
    let caller_uri = path_to_uri(&caller_str);
    let req = json!({
        "params": {
            "textDocument": { "uri": &src_uri },
            "position": { "line": 0, "character": 7 },
            "newName": "bar"
        }
    });

    let res = handle_rename(&req, Some(&ws), &overlay_mgr, &mut conv).expect("rename should succeed");
    let changes = res["changes"].as_object().expect("changes object");

    assert!(changes.contains_key(&src_uri));
    assert!(changes.contains_key(&caller_uri));

    let caller_edits = changes[&caller_uri].as_array().unwrap();
    assert_eq!(caller_edits.len(), 1);
    assert_eq!(caller_edits[0]["newText"], "bar");
    assert_eq!(caller_edits[0]["range"]["start"]["line"], 0);
    assert_eq!(caller_edits[0]["range"]["start"]["character"], 10);
    assert_eq!(caller_edits[0]["range"]["end"]["character"], 13);

    let _ = fs::remove_dir_all(&temp_dir);
}
