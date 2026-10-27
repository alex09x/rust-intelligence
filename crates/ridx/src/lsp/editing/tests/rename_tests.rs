/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::setup_test_editing;
use crate::lsp::editing::*;
use crate::overlay::OverlayManager;
use serde_json::{json, Value};
use std::fs;

#[test]
fn test_ident_and_keyword_validation() {
    assert!(is_valid_rust_ident("my_var"));
    assert!(is_valid_rust_ident("_hidden"));
    assert!(!is_valid_rust_ident(""));
    assert!(!is_valid_rust_ident("123num"));
    assert!(!is_valid_rust_ident("let"));
    assert!(!is_valid_rust_ident("fn"));
    assert!(!is_valid_rust_ident("async"));
    assert!(is_rust_keyword("match"));
    assert!(!is_rust_keyword("my_func"));
}

#[test]
fn test_find_declaration_ident_col() {
    let line = "pub fn add_values(x: i32, y: i32) -> i32 {";
    let col = find_declaration_ident_col(line, "add_values", ridx_format::DefKind::Fn);
    assert_eq!(col, Some(8));

    let line_struct = "pub struct Calculator {";
    let col2 = find_declaration_ident_col(line_struct, "Calculator", ridx_format::DefKind::Struct);
    assert_eq!(col2, Some(12));

    assert_eq!(
        find_declaration_ident_col("enum Color {", "Color", ridx_format::DefKind::Enum),
        Some(6)
    );
    assert_eq!(
        find_declaration_ident_col("pub trait Shape {", "Shape", ridx_format::DefKind::Trait),
        Some(11)
    );
    assert_eq!(
        find_declaration_ident_col(
            "pub type MyInt = i32;",
            "MyInt",
            ridx_format::DefKind::TyAlias
        ),
        Some(10)
    );
    assert_eq!(
        find_declaration_ident_col(
            "const MAX_LEN: usize = 10;",
            "MAX_LEN",
            ridx_format::DefKind::Const
        ),
        Some(7)
    );
    assert_eq!(
        find_declaration_ident_col(
            "static GLOBAL: i32 = 0;",
            "GLOBAL",
            ridx_format::DefKind::Static
        ),
        Some(8)
    );
    assert_eq!(
        find_declaration_ident_col("pub mod network;", "network", ridx_format::DefKind::Mod),
        Some(9)
    );
    assert_eq!(
        find_declaration_ident_col("name: String,", "name", ridx_format::DefKind::Field),
        Some(1)
    );
    assert_eq!(
        find_declaration_ident_col("start_ident();", "start_ident", ridx_format::DefKind::Fn),
        Some(1)
    );
    assert_eq!(
        find_declaration_ident_col("let end_ident", "end_ident", ridx_format::DefKind::Local),
        Some(5)
    );
    assert_eq!(
        find_declaration_ident_col("foo fn foo()", "foo", ridx_format::DefKind::Fn),
        Some(8)
    );
    assert_eq!(
        find_declaration_ident_col("foo fn bar()", "foo", ridx_format::DefKind::Fn),
        Some(1)
    );
}

#[test]
fn test_rename_and_prepare_rename() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_editing();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    let code = "pub fn add_values(x: i32, y: i32) -> i32 {\n    let sum = x + y;\n    sum\n}\n";
    overlay_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, code.to_string());

    // 1. Prepare rename
    let prep_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 9 }
        }
    });
    let prep_res = handle_prepare_rename(&prep_req, Some(&ws), &overlay_mgr, &mut conv)
        .expect("prepare rename succeeds");
    assert!(!prep_res.is_null());

    let empty_mgr = OverlayManager::new();
    let prep_disk_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 10 }
        }
    });
    let prep_disk = handle_prepare_rename(&prep_disk_req, Some(&ws), &empty_mgr, &mut conv)
        .expect("prepare rename succeeds");
    assert!(!prep_disk.is_null());

    let prep_none = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 0 }
        }
    });
    assert_eq!(
        handle_prepare_rename(&prep_none, Some(&ws), &overlay_mgr, &mut conv)
            .unwrap_err()
            .code,
        -32803
    );

    // 2. Rename local variable
    let rename_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 9 },
            "newName": "total"
        }
    });
    let rename_res =
        handle_rename(&rename_req, Some(&ws), &overlay_mgr, &mut conv).expect("rename succeeds");
    assert!(rename_res["changes"].get(&uri).is_some());

    // 3. Rename compiled symbol across workspace
    let rename_compiled = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 8 },
            "newName": "sum_values"
        }
    });
    let res_compiled = handle_rename(&rename_compiled, Some(&ws), &overlay_mgr, &mut conv)
        .expect("rename succeeds");
    assert!(res_compiled["changes"].get(&uri).is_some());

    let res_compiled_disk =
        handle_rename(&rename_compiled, Some(&ws), &empty_mgr, &mut conv).expect("rename succeeds");
    assert!(res_compiled_disk["changes"].get(&uri).is_some());

    // 4. Invalid rename ident -> RequestFailed error
    let invalid_rename = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 9 },
            "newName": "123invalid"
        }
    });
    let err_res = handle_rename(&invalid_rename, Some(&ws), &overlay_mgr, &mut conv);
    assert_eq!(err_res.unwrap_err().code, -32803);

    // 5. Linked editing range
    let linked_res = handle_linked_editing_range(&prep_req, &overlay_mgr, &mut conv);
    assert!(!linked_res.is_null());

    let caller_file = temp_dir.join("src/caller.rs");
    let caller_uri = format!("file://{}", caller_file.to_str().unwrap());
    let rename_from_caller = json!({
        "params": {
            "textDocument": { "uri": &caller_uri },
            "position": { "line": 0, "character": 20 },
            "newName": "sum_values"
        }
    });

    // 6. Rename with modified imports returns RequestFailed error
    overlay_mgr.update(
        file_path.to_str().unwrap(),
        2,
        "use std::collections::HashMap;\npub fn add_values(x: i32, y: i32) -> i32 { x + y }\n"
            .to_string(),
    );
    let res_dirty_imports = handle_rename(&rename_from_caller, Some(&ws), &overlay_mgr, &mut conv);
    assert_eq!(res_dirty_imports.unwrap_err().code, -32803);

    // 7. Rename when identifier at mapped target no longer matches
    let mut ov_decl_mismatch = OverlayManager::new();
    ov_decl_mismatch.open_document(
        &uri,
        file_path.to_str().unwrap(),
        1,
        "pub fn different_name(x: i32, y: i32) -> i32 { x + y }\n".to_string(),
    );
    let res_ident_mismatch =
        handle_rename(&rename_from_caller, Some(&ws), &ov_decl_mismatch, &mut conv);
    assert_eq!(res_ident_mismatch.unwrap_err().code, -32803);

    // 8. Rename when declaration is shifted in dirty buffer (maps to live line)
    overlay_mgr.update(file_path.to_str().unwrap(), 4, "// shifted\npub fn add_values(x: i32, y: i32) -> i32 {\n    let sum = x + y;\n    sum\n}\n".to_string());
    let res_shifted = handle_rename(&rename_from_caller, Some(&ws), &overlay_mgr, &mut conv)
        .expect("rename succeeds");
    assert!(!res_shifted["changes"].as_object().unwrap().is_empty());

    // 9. Rename when declaration line mapping returns None
    overlay_mgr.update(
        file_path.to_str().unwrap(),
        5,
        "top\nmod_mid\nbot\n".to_string(),
    );
    let res_unmapped = handle_rename(&rename_from_caller, Some(&ws), &overlay_mgr, &mut conv);
    assert_eq!(res_unmapped.unwrap_err().code, -32803);

    // 10. Linked editing range with overlay None (lines 345, 362)
    assert_eq!(
        handle_linked_editing_range(&prep_req, &empty_mgr, &mut conv),
        Value::Null
    );

    let rename_from_def = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 8 },
            "newName": "sum_values"
        }
    });

    // 11. Rename when caller overlay is dirty with modified imports
    let mut ov_caller_imports = OverlayManager::new();
    ov_caller_imports.open_document(
        &caller_uri,
        caller_file.to_str().unwrap(),
        1,
        "pub fn call() { test_edit::add_values(1, 2); }\n".to_string(),
    );
    ov_caller_imports.update(
        caller_file.to_str().unwrap(),
        2,
        "use std::collections::HashMap;\npub fn call() { test_edit::add_values(1, 2); }\n"
            .to_string(),
    );
    let res_caller_imports =
        handle_rename(&rename_from_def, Some(&ws), &ov_caller_imports, &mut conv);
    assert_eq!(res_caller_imports.unwrap_err().code, -32803);

    // 12. Rename when caller overlay has unmapped line
    let mut ov_caller_unmapped = OverlayManager::new();
    ov_caller_unmapped.open_document(
        &caller_uri,
        caller_file.to_str().unwrap(),
        1,
        "pub fn call() { test_edit::add_values(1, 2); }\n".to_string(),
    );
    ov_caller_unmapped.update(
        caller_file.to_str().unwrap(),
        2,
        "completely\ndifferent\nlines\n".to_string(),
    );
    let res_caller_unmapped =
        handle_rename(&rename_from_def, Some(&ws), &ov_caller_unmapped, &mut conv);
    assert_eq!(res_caller_unmapped.unwrap_err().code, -32803);

    // 13. Prepare rename on dirty overlay returns RequestFailed error
    let mut ov_dirty_prep = OverlayManager::new();
    ov_dirty_prep.open_document(
        &uri,
        file_path.to_str().unwrap(),
        1,
        "pub fn add_values(x: i32, y: i32) -> i32 { x + y }\n".to_string(),
    );
    ov_dirty_prep.update(
        file_path.to_str().unwrap(),
        2,
        "// dirty\npub fn add_values(x: i32, y: i32) -> i32 { x + y }\n".to_string(),
    );
    assert_eq!(
        handle_prepare_rename(&prep_req, Some(&ws), &ov_dirty_prep, &mut conv)
            .unwrap_err()
            .code,
        -32803
    );

    let _ = fs::remove_dir_all(&temp_dir);
}
