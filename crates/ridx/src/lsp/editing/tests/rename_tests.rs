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

    assert_eq!(find_declaration_ident_col("enum Color {", "Color", ridx_format::DefKind::Enum), Some(6));
    assert_eq!(find_declaration_ident_col("pub trait Shape {", "Shape", ridx_format::DefKind::Trait), Some(11));
    assert_eq!(find_declaration_ident_col("pub type MyInt = i32;", "MyInt", ridx_format::DefKind::TyAlias), Some(10));
    assert_eq!(find_declaration_ident_col("const MAX_LEN: usize = 10;", "MAX_LEN", ridx_format::DefKind::Const), Some(7));
    assert_eq!(find_declaration_ident_col("static GLOBAL: i32 = 0;", "GLOBAL", ridx_format::DefKind::Static), Some(8));
    assert_eq!(find_declaration_ident_col("pub mod network;", "network", ridx_format::DefKind::Mod), Some(9));
    assert_eq!(find_declaration_ident_col("name: String,", "name", ridx_format::DefKind::Field), Some(1));
    assert_eq!(find_declaration_ident_col("start_ident();", "start_ident", ridx_format::DefKind::Fn), Some(1));
    assert_eq!(find_declaration_ident_col("let end_ident", "end_ident", ridx_format::DefKind::Local), Some(5));
    assert_eq!(find_declaration_ident_col("foo fn foo()", "foo", ridx_format::DefKind::Fn), Some(8));
    assert_eq!(find_declaration_ident_col("foo fn bar()", "foo", ridx_format::DefKind::Fn), Some(1));
}

#[test]
fn test_rename_and_prepare_rename() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_editing();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = crate::lsp::protocol::path_to_uri(file_path.to_str().unwrap());

    let code = "pub fn add_values(x: i32, y: i32) -> i32 {\n    let sum = x + y;\n    sum\n}\n";
    overlay_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, code.to_string());

    // 1. Prepare rename
    let prep_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 9 }
        }
    });
    let prep_res = handle_prepare_rename(&prep_req, Some(&ws), &overlay_mgr, &mut conv).expect("prepare rename succeeds");
    assert!(!prep_res.is_null());

    let empty_mgr = OverlayManager::new();
    let prep_disk_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 10 }
        }
    });
    let prep_disk = handle_prepare_rename(&prep_disk_req, Some(&ws), &empty_mgr, &mut conv).expect("prepare rename succeeds");
    assert!(!prep_disk.is_null());

    let prep_none = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 0 }
        }
    });
    assert_eq!(handle_prepare_rename(&prep_none, Some(&ws), &overlay_mgr, &mut conv).unwrap_err().code, -32803);

    // 2. Rename local variable
    let rename_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 9 },
            "newName": "total"
        }
    });
    let rename_res = handle_rename(&rename_req, Some(&ws), &overlay_mgr, &mut conv).expect("rename succeeds");
    assert!(rename_res["changes"].get(&uri).is_some());

    // 3. Rename compiled symbol across workspace
    let rename_compiled = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 8 },
            "newName": "sum_values"
        }
    });
    let res_compiled = handle_rename(&rename_compiled, Some(&ws), &overlay_mgr, &mut conv).expect("rename succeeds");
    assert!(res_compiled["changes"].get(&uri).is_some());

    let res_compiled_disk = handle_rename(&rename_compiled, Some(&ws), &empty_mgr, &mut conv).expect("rename succeeds");
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
    let caller_uri = crate::lsp::protocol::path_to_uri(caller_file.to_str().unwrap());
    let rename_from_caller = json!({
        "params": {
            "textDocument": { "uri": &caller_uri },
            "position": { "line": 0, "character": 20 },
            "newName": "sum_values"
        }
    });

    // 6. Rename with modified imports returns RequestFailed error
    overlay_mgr.update(file_path.to_str().unwrap(), 2, "use std::collections::HashMap;\npub fn add_values(x: i32, y: i32) -> i32 { x + y }\n".to_string());
    let res_dirty_imports = handle_rename(&rename_from_caller, Some(&ws), &overlay_mgr, &mut conv);
    assert_eq!(res_dirty_imports.unwrap_err().code, -32803);

    // 7. Rename when identifier at mapped target no longer matches
    let mut ov_decl_mismatch = OverlayManager::new();
    ov_decl_mismatch.open_document(&uri, file_path.to_str().unwrap(), 1, "pub fn different_name(x: i32, y: i32) -> i32 { x + y }\n".to_string());
    let res_ident_mismatch = handle_rename(&rename_from_caller, Some(&ws), &ov_decl_mismatch, &mut conv);
    assert_eq!(res_ident_mismatch.unwrap_err().code, -32803);

    // 8. Rename when declaration is shifted in dirty buffer (maps to live line)
    overlay_mgr.update(file_path.to_str().unwrap(), 4, "// shifted\npub fn add_values(x: i32, y: i32) -> i32 {\n    let sum = x + y;\n    sum\n}\n".to_string());
    let res_shifted = handle_rename(&rename_from_caller, Some(&ws), &overlay_mgr, &mut conv).expect("rename succeeds");
    assert!(!res_shifted["changes"].as_object().unwrap().is_empty());

    // 9. Rename when declaration line mapping returns None
    overlay_mgr.update(file_path.to_str().unwrap(), 5, "top\nmod_mid\nbot\n".to_string());
    let res_unmapped = handle_rename(&rename_from_caller, Some(&ws), &overlay_mgr, &mut conv);
    assert_eq!(res_unmapped.unwrap_err().code, -32803);

    // 10. Linked editing range with overlay None (lines 345, 362)
    assert_eq!(handle_linked_editing_range(&prep_req, &empty_mgr, &mut conv), Value::Null);

    let rename_from_def = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 8 },
            "newName": "sum_values"
        }
    });

    // 11. A caller with an import added since: one of another name (`HashMap`) changes nothing for `add_values`, the
    // call is renamed at its shifted line; an import of `add_values` itself may change what it resolves to: refused.
    let mut ov_caller_imports = OverlayManager::new();
    ov_caller_imports.open_document(&caller_uri, caller_file.to_str().unwrap(), 1, "pub fn call() { test_edit::add_values(1, 2); }\n".to_string());
    ov_caller_imports.update(caller_file.to_str().unwrap(), 2, "use std::collections::HashMap;\npub fn call() { test_edit::add_values(1, 2); }\n".to_string());
    let res_caller_imports = handle_rename(&rename_from_def, Some(&ws), &ov_caller_imports, &mut conv).expect("an unrelated import does not block");
    let caller_edits = res_caller_imports["changes"][caller_uri.as_str()].as_array().cloned().unwrap_or_default();
    assert!(caller_edits.iter().all(|e| e["range"]["start"]["line"] == 1), "the call on its shifted line: {res_caller_imports}");
    ov_caller_imports.update(caller_file.to_str().unwrap(), 3, "use other::add_values;\npub fn call() { test_edit::add_values(1, 2); }\n".to_string());
    let res_caller_imports = handle_rename(&rename_from_def, Some(&ws), &ov_caller_imports, &mut conv);
    assert_eq!(res_caller_imports.unwrap_err().code, -32803);

    // 12. Rename when caller overlay has unmapped line
    let mut ov_caller_unmapped = OverlayManager::new();
    ov_caller_unmapped.open_document(&caller_uri, caller_file.to_str().unwrap(), 1, "pub fn call() { test_edit::add_values(1, 2); }\n".to_string());
    ov_caller_unmapped.update(caller_file.to_str().unwrap(), 2, "completely\ndifferent\nlines\n".to_string());
    let res_caller_unmapped = handle_rename(&rename_from_def, Some(&ws), &ov_caller_unmapped, &mut conv);
    assert_eq!(res_caller_unmapped.unwrap_err().code, -32803);

    // 13. Prepare rename on dirty overlay returns RequestFailed error
    let mut ov_dirty_prep = OverlayManager::new();
    ov_dirty_prep.open_document(&uri, file_path.to_str().unwrap(), 1, "pub fn add_values(x: i32, y: i32) -> i32 { x + y }\n".to_string());
    ov_dirty_prep.update(file_path.to_str().unwrap(), 2, "// dirty\npub fn add_values(x: i32, y: i32) -> i32 { x + y }\n".to_string());
    assert_eq!(handle_prepare_rename(&prep_req, Some(&ws), &ov_dirty_prep, &mut conv).unwrap_err().code, -32803);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_rename_special_shorthand_lifetime_alias_and_keyword_refusal() {
    use crate::lsp::editing::rename_special::{field_edit_new_text, local_edit_new_text};
    use crate::lsp::editing::rename_unindexed::detect_module_file_rename;

    let lines = vec!["pub struct Foo { pub count: i32 }", "pub fn make(count: i32) -> Foo {", "    Foo { count }", "}"];
    assert_eq!(field_edit_new_text(&lines, 1, 22, 27, true, "count", "total"), "total");
    assert_eq!(field_edit_new_text(&lines, 3, 11, 16, false, "count", "total"), "total: count");
    assert_eq!(local_edit_new_text(&lines, 3, 11, 16, "count", "val"), "count: val");
    assert_eq!(local_edit_new_text(&lines, 2, 13, 18, "count", "val"), "val");

    let temp_dir = std::env::temp_dir().join(format!("ridx_special_rename_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(temp_dir.join("src")).unwrap();
    let lib_file = temp_dir.join("src/lib.rs");
    let sub_file = temp_dir.join("src/sub_mod.rs");
    fs::write(&sub_file, "pub struct Orig;\n").unwrap();
    let src_text = "pub mod sub_mod;\nuse crate::sub_mod::Orig as MyAlias;\npub fn borrow<'a>(s: &'a str) -> &'a str { let _x = MyAlias; s }\n";
    fs::write(&lib_file, src_text).unwrap();
    let lib_str = lib_file.to_str().unwrap();
    let lib_uri = crate::lsp::protocol::path_to_uri(lib_str);
    let ov_mgr = OverlayManager::new();
    let mut conv = crate::lsp::protocol::PositionConverter::new();

    // 1. Keyword refusal on `pub` / `fn`
    let kw_req = json!({ "params": { "textDocument": { "uri": &lib_uri }, "position": { "line": 2, "character": 4 }, "newName": "renamed" } });
    assert!(handle_rename(&kw_req, None, &ov_mgr, &mut conv).is_err());

    // 2. Lifetime `'a` -> `'b`
    let lt_req = json!({ "params": { "textDocument": { "uri": &lib_uri }, "position": { "line": 2, "character": 14 }, "newName": "'b" } });
    let lt_res = handle_rename(&lt_req, None, &ov_mgr, &mut conv).expect("lifetime rename succeeds");
    assert_eq!(lt_res["changes"][&lib_uri].as_array().unwrap().len(), 3);
    assert_eq!(lt_res["changes"][&lib_uri][0]["newText"], "'b");

    // 3. `use ... as` alias rename (`MyAlias` -> `NewAlias`)
    let alias_req = json!({ "params": { "textDocument": { "uri": &lib_uri }, "position": { "line": 1, "character": 29 }, "newName": "NewAlias" } });
    let alias_res = handle_rename(&alias_req, None, &ov_mgr, &mut conv).expect("alias rename succeeds");
    assert_eq!(alias_res["changes"][&lib_uri].as_array().unwrap().len(), 2);

    // 4. Module file rename detection (`pub mod sub_mod;`)
    let src_lines: Vec<&str> = src_text.lines().collect();
    let mod_ren = detect_module_file_rename(lib_str, &src_lines, 1, "sub_mod", "renamed_mod").expect("detects sub_mod.rs");
    assert!(mod_ren.0.ends_with("/src/sub_mod.rs"));
    assert!(mod_ren.1.ends_with("/src/renamed_mod.rs"));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_rename_separates_same_path_field_vs_assoc_fn_and_ty_param_vs_assoc_ty() {
    use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference, WorkspaceIndex};

    let temp_dir = std::env::temp_dir().join(format!("ridx_same_path_rename_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(temp_dir.join("src")).unwrap();
    fs::create_dir_all(temp_dir.join("idx")).unwrap();
    let src_file = temp_dir.join("src/lib.rs");
    let src_str = src_file.to_str().unwrap();
    let src_code = concat!(
        "pub struct Handle {\n",
        "    pub time: u64,\n",
        "}\n",
        "impl Handle {\n",
        "    pub fn time(&self) -> u64 {\n",
        "        self.time\n",
        "    }\n",
        "}\n",
        "pub fn use_handle(h: &Handle) -> u64 { h.time() }\n",
        "pub trait Seq { type Error; }\n",
        "pub struct Imp<Error>(Error);\n",
        "impl<Error> Seq for Imp<Error> {\n",
        "    type Error = Error;\n",
        "}\n",
    );
    fs::write(&src_file, src_code).unwrap();

    for (idx_name, unit) in [("lib.ridx", "pkg_lib"), ("test.ridx", "pkg_test")] {
        let mut w = IndexWriter::new("pkg", "0.1.0");
        w.set_unit_key(unit);
        w.add_definition(RawDefinition {
            name: "time".into(),
            path: "pkg::Handle::time".into(),
            kind: DefKind::Field,
            file: src_str.into(),
            start_line: 2,
            start_col: 9,
            end_line: 2,
            end_col: 13,
            signature: None,
            def_id: 1,
            visibility: 0,
            is_macro_expansion: false,
        });
        w.add_definition(RawDefinition {
            name: "time".into(),
            path: "pkg::Handle::time".into(),
            kind: DefKind::AssocFn,
            file: src_str.into(),
            start_line: 5,
            start_col: 12,
            end_line: 5,
            end_col: 16,
            signature: Some("pub fn time(&self) -> u64".into()),
            def_id: 2,
            visibility: 0,
            is_macro_expansion: false,
        });
        w.add_reference(RawReference {
            file: src_str.into(),
            start_line: 6,
            start_col: 14,
            end_line: 6,
            end_col: 18,
            target_def_id: Some(1),
            target_path: "pkg::Handle::time".into(),
            target_unit: Some(unit.into()),
            target_file: Some(src_str.into()),
            target_line: 2,
            target_col: 9,
            kind: DefKind::Field,
            is_write: false,
            is_macro_expansion: false,
        });
        w.add_reference(RawReference {
            file: src_str.into(),
            start_line: 9,
            start_col: 42,
            end_line: 9,
            end_col: 46,
            target_def_id: Some(2),
            target_path: "pkg::Handle::time".into(),
            target_unit: Some(unit.into()),
            target_file: Some(src_str.into()),
            target_line: 5,
            target_col: 12,
            kind: DefKind::AssocFn,
            is_write: false,
            is_macro_expansion: false,
        });
        w.add_definition(RawDefinition {
            name: "Error".into(),
            path: "<pkg::Imp<Error> as pkg::Seq>::Error".into(),
            kind: DefKind::TyParam,
            file: src_str.into(),
            start_line: 12,
            start_col: 6,
            end_line: 12,
            end_col: 11,
            signature: None,
            def_id: 3,
            visibility: 0,
            is_macro_expansion: false,
        });
        w.add_definition(RawDefinition {
            name: "Error".into(),
            path: "<pkg::Imp<Error> as pkg::Seq>::Error".into(),
            kind: DefKind::AssocTy,
            file: src_str.into(),
            start_line: 13,
            start_col: 10,
            end_line: 13,
            end_col: 15,
            signature: None,
            def_id: 4,
            visibility: 0,
            is_macro_expansion: false,
        });
        w.add_reference(RawReference {
            file: src_str.into(),
            start_line: 13,
            start_col: 18,
            end_line: 13,
            end_col: 23,
            target_def_id: Some(3),
            target_path: "<pkg::Imp<Error> as pkg::Seq>::Error".into(),
            target_unit: Some(unit.into()),
            target_file: Some(src_str.into()),
            target_line: 12,
            target_col: 6,
            kind: DefKind::TyParam,
            is_write: false,
            is_macro_expansion: false,
        });
        let mut f = fs::File::create(temp_dir.join("idx").join(idx_name)).unwrap();
        w.write_to(&mut f).unwrap();
    }

    let mut ws = WorkspaceIndex::open(temp_dir.join("idx")).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let fn_refs = ws.find_rename_references_at(src_str, 5, 12).expect("resolves AssocFn time");
    assert_eq!(fn_refs.target_kind, DefKind::AssocFn);
    assert!(fn_refs.references.iter().all(|r| r.span.start_line() == 9));

    let assoc_ty_refs = ws.find_rename_references_at(src_str, 13, 10).expect("resolves AssocTy Error");
    assert_eq!(assoc_ty_refs.target_kind, DefKind::AssocTy);
    assert!(assoc_ty_refs.references.iter().all(|r| r.kind != DefKind::TyParam));

    let _ = fs::remove_dir_all(&temp_dir);
}
