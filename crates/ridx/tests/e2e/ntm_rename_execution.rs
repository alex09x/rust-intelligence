/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use ridx::lsp::editing::rename::handle_rename;
use ridx::lsp::navigation::handle_definition;
use ridx::lsp::protocol::PositionConverter;
use ridx::overlay::OverlayManager;
use ridx_format::{IndexWriter, RawDefinition, RawReference, WorkspaceIndex};
use ridx_test_support::apply_text_edits;
use serde_json::json;
use std::fs;

#[test]
fn test_refactor_rename_local_variable_execution() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = "file:///workspace/src/counter.rs";
    let file_path = "/workspace/src/counter.rs";

    let original_code = concat!(
        "fn process_data(items: &[i32]) -> i32 {\n",
        "    let mut count = 0;\n",
        "    for _x in items {\n",
        "        count += 1;\n",
        "    }\n",
        "    let text = \"count in string\";\n",
        "    // count in comments\n",
        "    count\n",
        "}\n",
    );

    overlay_mgr.open_document(uri, file_path, 1, original_code.to_string());

    let rename_req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": 1, "character": 13 },
            "newName": "total_items"
        }
    });

    let res = handle_rename(&rename_req, None, &overlay_mgr, &mut conv).expect("rename succeeds");
    let edits = res["changes"][uri].as_array().expect("edits array");
    assert!(!edits.is_empty(), "rename must return edits");

    let transformed = apply_text_edits(original_code, edits);

    assert!(transformed.contains("let mut total_items = 0;"));
    assert!(transformed.contains("total_items += 1;"));
    assert!(transformed.contains("    total_items\n"));
    assert!(transformed.contains("\"count in string\""));
    assert!(transformed.contains("// count in comments"));
    assert!(!transformed.contains("let mut count = 0;"));
}

#[test]
fn test_refactor_cross_file_workspace_rename_execution() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_cross_rename_{}", std::process::id()));
    let _ = fs::create_dir_all(temp_dir.join("src"));

    let def_file = temp_dir.join("src/lib.rs");
    let def_code = "pub fn add_values(x: i32, y: i32) -> i32 {\n    x + y\n}\n";
    let _ = fs::write(&def_file, def_code);

    let caller_file = temp_dir.join("src/caller.rs");
    let caller_code = "pub fn call() -> i32 {\n    test_pkg::add_values(1, 2)\n}\n";
    let _ = fs::write(&caller_file, caller_code);

    let def_file_str = def_file.to_str().unwrap().to_string();
    let caller_file_str = caller_file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("test_pkg", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: ridx_format::DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: def_file_str.clone(),
        name: "add_values".to_string(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 18,
        path: "test_pkg::add_values".to_string(),
        signature: Some("fn add_values(x: i32, y: i32) -> i32".to_string()),
    });

    writer.add_reference(RawReference {
        file: def_file_str.clone(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 18,
        target_def_id: Some(1),
        target_path: "test_pkg::add_values".to_string(),
        target_unit: None,
        target_file: Some(def_file_str.clone()),
        target_line: 1,
        target_col: 8,
        kind: ridx_format::DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });

    writer.add_reference(RawReference {
        file: caller_file_str.clone(),
        start_line: 2,
        start_col: 15,
        end_line: 2,
        end_col: 25,
        target_def_id: Some(1),
        target_path: "test_pkg::add_values".to_string(),
        target_unit: None,
        target_file: Some(def_file_str.clone()),
        target_line: 1,
        target_col: 8,
        kind: ridx_format::DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });

    let index_path = temp_dir.join("index.ridx");
    let mut f = fs::File::create(&index_path).unwrap();
    writer.write_to(&mut f).unwrap();

    let ws = WorkspaceIndex::open(&index_path).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let def_uri = format!("file://{}", def_file_str);
    let caller_uri = format!("file://{}", caller_file_str);

    let rename_req = json!({
        "params": {
            "textDocument": { "uri": &def_uri },
            "position": { "line": 0, "character": 8 },
            "newName": "sum_values"
        }
    });

    let res = handle_rename(&rename_req, Some(&ws), &overlay_mgr, &mut conv).expect("rename succeeds");
    let changes = res["changes"].as_object().expect("changes object");

    let def_edits = changes.get(&def_uri).expect("edits for lib.rs").as_array().unwrap();
    let caller_edits = changes.get(&caller_uri).expect("edits for caller.rs").as_array().unwrap();

    let new_def_code = apply_text_edits(def_code, def_edits);
    let new_caller_code = apply_text_edits(caller_code, caller_edits);

    assert_eq!(new_def_code, "pub fn sum_values(x: i32, y: i32) -> i32 {\n    x + y\n}\n");
    assert_eq!(new_caller_code, "pub fn call() -> i32 {\n    test_pkg::sum_values(1, 2)\n}\n");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_goto_def_and_rename_generic_type_param_t() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_typaram_t_{}", std::process::id()));
    let _ = fs::create_dir_all(temp_dir.join("src"));

    let def_file = temp_dir.join("src/lib.rs");
    let def_code = "pub fn process<T: Clone>(item: T) -> T {\n    item.clone()\n}\n";
    let _ = fs::write(&def_file, def_code);

    let def_file_str = def_file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("test_pkg", "0.1.0");
    // Item 1: fn process (start_line: 1, start_col: 8, end_line: 1, end_col: 15)
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: ridx_format::DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: def_file_str.clone(),
        name: "process".to_string(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 15,
        path: "test_pkg::process".to_string(),
        signature: Some("fn process<T: Clone>(item: T) -> T".to_string()),
    });

    // Item 2: TyParam T (exact identifier span: line 1, col 16..17)
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: ridx_format::DefKind::TyParam,
        visibility: 0,
        is_macro_expansion: false,
        file: def_file_str.clone(),
        name: "T".to_string(),
        start_line: 1,
        start_col: 16,
        end_line: 1,
        end_col: 17,
        path: "test_pkg::process::T".to_string(),
        signature: Some("Clone".to_string()),
    });

    // Self-reference at declaration site
    writer.add_reference(RawReference {
        file: def_file_str.clone(),
        start_line: 1,
        start_col: 16,
        end_line: 1,
        end_col: 17,
        target_def_id: Some(2),
        target_path: "test_pkg::process::T".to_string(),
        target_unit: None,
        target_file: Some(def_file_str.clone()),
        target_line: 1,
        target_col: 16,
        kind: ridx_format::DefKind::TyParam,
        is_write: false,
        is_macro_expansion: false,
    });

    // Reference at item: T (line 1, col 32..33)
    writer.add_reference(RawReference {
        file: def_file_str.clone(),
        start_line: 1,
        start_col: 32,
        end_line: 1,
        end_col: 33,
        target_def_id: Some(2),
        target_path: "test_pkg::process::T".to_string(),
        target_unit: None,
        target_file: Some(def_file_str.clone()),
        target_line: 1,
        target_col: 16,
        kind: ridx_format::DefKind::TyParam,
        is_write: false,
        is_macro_expansion: false,
    });

    // Reference at -> T (line 1, col 38..39)
    writer.add_reference(RawReference {
        file: def_file_str.clone(),
        start_line: 1,
        start_col: 38,
        end_line: 1,
        end_col: 39,
        target_def_id: Some(2),
        target_path: "test_pkg::process::T".to_string(),
        target_unit: None,
        target_file: Some(def_file_str.clone()),
        target_line: 1,
        target_col: 16,
        kind: ridx_format::DefKind::TyParam,
        is_write: false,
        is_macro_expansion: false,
    });

    let index_path = temp_dir.join("index.ridx");
    let mut f = fs::File::create(&index_path).unwrap();
    writer.write_to(&mut f).unwrap();

    let ws = WorkspaceIndex::open(&index_path).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let def_uri = format!("file://{}", def_file_str);

    // 1. Verify Goto Definition on `T` at `item: T` (line 0, character 31)
    let goto_req = json!({
        "params": {
            "textDocument": { "uri": &def_uri },
            "position": { "line": 0, "character": 31 }
        }
    });
    let goto_res = handle_definition(&goto_req, Some(&ws), &overlay_mgr, &mut conv);
    assert_eq!(goto_res["range"]["start"]["line"], 0, "Goto-def must land on line 0");
    assert_eq!(goto_res["range"]["start"]["character"], 15, "Goto-def on T must land on T identifier (character 15), NOT on function start");
    assert_eq!(goto_res["range"]["end"]["character"], 16, "Goto-def on T must end at character 16");

    // 2. Verify Rename `T` -> `ItemTy`
    let rename_req = json!({
        "params": {
            "textDocument": { "uri": &def_uri },
            "position": { "line": 0, "character": 15 },
            "newName": "ItemTy"
        }
    });
    let rename_res = handle_rename(&rename_req, Some(&ws), &overlay_mgr, &mut conv).expect("rename succeeds");
    let changes = rename_res["changes"].as_object().expect("changes object");
    let def_edits = changes.get(&def_uri).expect("edits for lib.rs").as_array().unwrap();

    assert_eq!(def_edits.len(), 3, "All 3 occurrences of T must be renamed");
    let new_code = apply_text_edits(def_code, def_edits);
    assert_eq!(new_code, "pub fn process<ItemTy: Clone>(item: ItemTy) -> ItemTy {\n    item.clone()\n}\n", "Rename must preserve function name and replace all T with ItemTy");

    let _ = fs::remove_dir_all(&temp_dir);
}
