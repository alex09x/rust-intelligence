/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compiler-grounded L2 integration tests for Domain 9: Navigation & Symbols.
//! Tests definition, declaration, and references on real fixtures covering
//! re-exports (FLAG_REEXPORT), macro expansions, raw identifiers, cross-crate dispatch,
//! and sysroot operations.

use super::l2_harness::{fixture_dir, get_or_index_l2_fixture, pos_in_file};
use ridx::lsp::navigation::references::handle_references;
use ridx::lsp::navigation::{handle_declaration, handle_definition};
use ridx::lsp::protocol::{path_to_uri, PositionConverter};
use ridx::overlay::OverlayManager;
use serde_json::{json, Value};

fn nav_req(uri: &str, line: u32, col: u32) -> Value {
    json!({
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": col }
        }
    })
}

#[test]
fn test_l2_reexport_item_definition_and_declaration() {
    let ws = get_or_index_l2_fixture("shorthand_and_reexports");
    let lib_file = fixture_dir("shorthand_and_reexports").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let (cur_line, cur_col) = pos_in_file(&lib_file, "construct_widget(\"example\")", "construct_widget");
    let req = nav_req(&uri, cur_line, cur_col);
    let def_res = handle_definition(&req, Some(ws), &overlay_mgr, &mut conv);
    let decl_res = handle_declaration(&req, Some(ws), &overlay_mgr, &mut conv);
    assert!(!def_res.is_null(), "Re-exported function call must resolve to definition");
    assert_eq!(def_res, decl_res, "Declaration must match definition");

    let (target_line, target_col) = pos_in_file(&lib_file, "pub fn construct_widget", "construct_widget");
    assert_eq!(def_res["range"]["start"]["line"], target_line);
    assert_eq!(def_res["range"]["start"]["character"], target_col);
}

#[test]
fn test_l2_reexport_statement_navigation() {
    let ws = get_or_index_l2_fixture("shorthand_and_reexports");
    let lib_file = fixture_dir("shorthand_and_reexports").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let (use_line, use_col) = pos_in_file(&lib_file, "pub use inner::construct_widget;", "construct_widget");
    let req = nav_req(&uri, use_line, use_col);
    let def_res = handle_definition(&req, Some(ws), &overlay_mgr, &mut conv);
    let decl_res = handle_declaration(&req, Some(ws), &overlay_mgr, &mut conv);
    assert!(!def_res.is_null());
    assert_eq!(def_res, decl_res);
}

#[test]
fn test_l2_reexport_aliased_type_navigation() {
    let ws = get_or_index_l2_fixture("shorthand_and_reexports");
    let lib_file = fixture_dir("shorthand_and_reexports").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let (cur_line, cur_col) = pos_in_file(&lib_file, "pub fn invoke_aliased_widget() -> AliasedWidget", "AliasedWidget");
    let req = nav_req(&uri, cur_line, cur_col);
    let def_res = handle_definition(&req, Some(ws), &overlay_mgr, &mut conv);
    let decl_res = handle_declaration(&req, Some(ws), &overlay_mgr, &mut conv);
    assert!(!def_res.is_null(), "Aliased re-export must resolve to original type");
    assert_eq!(def_res, decl_res);

    let (orig_line, orig_col) = pos_in_file(&lib_file, "pub struct InnerWidget", "InnerWidget");
    assert_eq!(def_res["range"]["start"]["line"], orig_line);
    assert_eq!(def_res["range"]["start"]["character"], orig_col);
}

#[test]
fn test_l2_raw_ident_navigation_and_references() {
    let ws = get_or_index_l2_fixture("shorthand_and_reexports");
    let lib_file = fixture_dir("shorthand_and_reexports").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let (cur_line, cur_col) = pos_in_file(&lib_file, "r#match(42)", "match");
    let req = nav_req(&uri, cur_line, cur_col);
    let def_res = handle_definition(&req, Some(ws), &overlay_mgr, &mut conv);
    let decl_res = handle_declaration(&req, Some(ws), &overlay_mgr, &mut conv);
    assert!(!def_res.is_null());
    assert_eq!(def_res, decl_res);

    let (fn_line, fn_col) = pos_in_file(&lib_file, "pub fn r#match", "r#match");
    assert_eq!(def_res["range"]["start"]["line"], fn_line);
    assert_eq!(def_res["range"]["start"]["character"], fn_col);

    // Query references on r#match definition (line fn_line, col fn_col)
    let ref_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": fn_line, "character": fn_col },
            "context": { "includeDeclaration": true }
        }
    });
    let ref_res = handle_references(&ref_req, Some(ws), &overlay_mgr, &mut conv);
    let arr = ref_res.as_array().expect("references array");
    assert_eq!(arr.len(), 2, "Must find exactly 2 references for r#match (declaration and call site)");
    assert_eq!(arr[0]["range"]["start"]["line"], fn_line);
    assert_eq!(arr[1]["range"]["start"]["line"], cur_line);

    // With includeDeclaration = false: exactly 1 reference (the call site)
    let ref_req_no_decl = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": fn_line, "character": fn_col },
            "context": { "includeDeclaration": false }
        }
    });
    let ref_res_no_decl = handle_references(&ref_req_no_decl, Some(ws), &overlay_mgr, &mut conv);
    let arr_no_decl = ref_res_no_decl.as_array().expect("references array");
    assert_eq!(arr_no_decl.len(), 1);
    assert_eq!(arr_no_decl[0]["range"]["start"]["line"], cur_line);
}

#[test]
fn test_l2_struct_field_shorthand_navigation_and_refs() {
    let ws = get_or_index_l2_fixture("shorthand_and_reexports");
    let lib_file = fixture_dir("shorthand_and_reexports").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let (cur_line, cur_col) = pos_in_file(&lib_file, "pub fn make_point(x: i32, y: i32) -> Point {\n    Point { x, y }", "y }");
    let req = nav_req(&uri, cur_line, cur_col);
    let def_res = handle_definition(&req, Some(ws), &overlay_mgr, &mut conv);
    let decl_res = handle_declaration(&req, Some(ws), &overlay_mgr, &mut conv);
    assert!(!def_res.is_null());
    assert_eq!(def_res, decl_res);
}

#[test]
fn test_l2_macro_multiplier_navigation_and_refs() {
    let ws = get_or_index_l2_fixture("macros_and_derives");
    let lib_file = fixture_dir("macros_and_derives").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let (cur_line, cur_col) = pos_in_file(&lib_file, "triple(10)", "triple");
    let req = nav_req(&uri, cur_line, cur_col);
    let def_res = handle_definition(&req, Some(ws), &overlay_mgr, &mut conv);
    let decl_res = handle_declaration(&req, Some(ws), &overlay_mgr, &mut conv);
    assert!(!def_res.is_null());
    assert_eq!(def_res, decl_res);

    let (macro_line, macro_col) = pos_in_file(&lib_file, "make_multiplier!(triple, 3);", "triple");
    assert_eq!(def_res["range"]["start"]["line"], macro_line);
    assert_eq!(def_res["range"]["start"]["character"], macro_col);

    let (q_line, q_col) = pos_in_file(&lib_file, "quadruple(5)", "quadruple");
    let q_req = nav_req(&uri, q_line, q_col);
    let q_def = handle_definition(&q_req, Some(ws), &overlay_mgr, &mut conv);
    let q_decl = handle_declaration(&q_req, Some(ws), &overlay_mgr, &mut conv);
    assert!(!q_def.is_null());
    assert_eq!(q_def, q_decl);

    let q_ref_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": q_line, "character": q_col },
            "context": { "includeDeclaration": true }
        }
    });
    let q_refs = handle_references(&q_ref_req, Some(ws), &overlay_mgr, &mut conv);
    let refs_arr = q_refs.as_array().expect("references array");
    let (macro_inv_line, _) = pos_in_file(&lib_file, "make_multiplier!(quadruple, 4);", "quadruple");
    assert_eq!(refs_arr.len(), 2, "Must find exactly 2 references (macro declaration site and call site)");
    assert_eq!(refs_arr[0]["range"]["start"]["line"], macro_inv_line);
    assert_eq!(refs_arr[1]["range"]["start"]["line"], q_line);
}

#[test]
fn test_l2_macro_included_file_navigation() {
    let ws = get_or_index_l2_fixture("macros_and_derives");
    let lib_file = fixture_dir("macros_and_derives").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let (inc_line, inc_col) = pos_in_file(&lib_file, "included_helper()", "included_helper");
    let inc_req = nav_req(&uri, inc_line, inc_col);
    let def_res = handle_definition(&inc_req, Some(ws), &overlay_mgr, &mut conv);
    let decl_res = handle_declaration(&inc_req, Some(ws), &overlay_mgr, &mut conv);
    assert!(!def_res.is_null());
    assert_eq!(def_res, decl_res);
    assert!(def_res["uri"].as_str().unwrap().ends_with("included.rs"));
}

#[test]
fn test_l2_cross_crate_dispatch_and_references() {
    let ws = get_or_index_l2_fixture("multi_crate");
    let app_file = fixture_dir("multi_crate").join("crates/app_crate/src/lib.rs");
    let app_uri = path_to_uri(app_file.to_str().unwrap());
    let core_file = fixture_dir("multi_crate").join("crates/core_lib/src/lib.rs");
    let core_uri = path_to_uri(core_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let (import_line, import_col) = pos_in_file(&app_file, "use core_lib::CoreService as ExternalCoreService;", "CoreService");
    let import_req = nav_req(&app_uri, import_line, import_col);
    let def_res = handle_definition(&import_req, Some(ws), &overlay_mgr, &mut conv);
    let decl_res = handle_declaration(&import_req, Some(ws), &overlay_mgr, &mut conv);
    assert!(!def_res.is_null());
    assert_eq!(def_res, decl_res);
    assert_eq!(def_res["uri"], core_uri);

    let (call_line, call_col) = pos_in_file(&app_file, "ext_svc.execute()", "execute");
    let call_req = nav_req(&app_uri, call_line, call_col);
    let call_def = handle_definition(&call_req, Some(ws), &overlay_mgr, &mut conv);
    let call_decl = handle_declaration(&call_req, Some(ws), &overlay_mgr, &mut conv);
    assert!(!call_def.is_null());
    assert_eq!(call_def, call_decl);
    assert_eq!(call_def["uri"], core_uri);

    let (core_exec_line, core_exec_col) = pos_in_file(&core_file, "pub fn execute(&self)", "execute");
    let ref_req = json!({
        "params": {
            "textDocument": { "uri": &core_uri },
            "position": { "line": core_exec_line, "character": core_exec_col },
            "context": { "includeDeclaration": true }
        }
    });
    let ref_res = handle_references(&ref_req, Some(ws), &overlay_mgr, &mut conv);
    let arr = ref_res.as_array().expect("references array");
    let has_app_call = arr.iter().any(|r| r["uri"] == app_uri);
    assert!(has_app_call, "Must find cross-crate call site in app_crate");
}

#[test]
fn test_l2_sysroot_ops_range_navigation() {
    let ws = get_or_index_l2_fixture("shorthand_and_reexports");
    let lib_file = fixture_dir("shorthand_and_reexports").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let (range_line, range_col) = pos_in_file(&lib_file, "let _ = ops::Range { start: 0, end: 1 };", "Range");
    let req = nav_req(&uri, range_line, range_col);
    let def_res = handle_definition(&req, Some(ws), &overlay_mgr, &mut conv);
    let decl_res = handle_declaration(&req, Some(ws), &overlay_mgr, &mut conv);
    assert!(!def_res.is_null(), "Sysroot ops::Range must resolve");
    assert_eq!(def_res, decl_res);
}

#[test]
fn test_l2_traits_call_generic_method_definition() {
    let ws = get_or_index_l2_fixture("traits_and_generics");
    let lib_file = fixture_dir("traits_and_generics").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Call: g.greet() in call_generic
    let (cur_line, cur_col) = pos_in_file(&lib_file, "g.greet()", "greet");
    let req = nav_req(&uri, cur_line, cur_col);
    let def_res = handle_definition(&req, Some(ws), &overlay_mgr, &mut conv);
    let decl_res = handle_declaration(&req, Some(ws), &overlay_mgr, &mut conv);
    assert!(!def_res.is_null());
    assert_eq!(def_res, decl_res);

    let (trait_line, trait_col) = pos_in_file(&lib_file, "fn greet(&self) -> &'static str;", "greet");
    assert_eq!(def_res["range"]["start"]["line"], trait_line);
    assert_eq!(def_res["range"]["start"]["character"], trait_col);
}

#[test]
fn test_l2_traits_default_method_references() {
    let ws = get_or_index_l2_fixture("traits_and_generics");
    let lib_file = fixture_dir("traits_and_generics").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let (def_line, def_col) = pos_in_file(&lib_file, "fn default_greet(&self)", "default_greet");
    let ref_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": def_line, "character": def_col },
            "context": { "includeDeclaration": true }
        }
    });

    let ref_res = handle_references(&ref_req, Some(ws), &overlay_mgr, &mut conv);
    let arr = ref_res.as_array().expect("references array");
    let (generic_line, _) = pos_in_file(&lib_file, "let _ = G::default_greet;", "default_greet");
    let (call_line, _) = pos_in_file(&lib_file, "r.default_greet()", "default_greet");
    let (wrap_line, _) = pos_in_file(&lib_file, "let _ = W::default_greet;", "default_greet");
    assert_eq!(arr.len(), 4, "Must find exactly 4 references: trait declaration, generic access, call_default, and wrap_greet");
    assert_eq!(arr[0]["range"]["start"]["line"], def_line);
    assert_eq!(arr[1]["range"]["start"]["line"], generic_line);
    assert_eq!(arr[2]["range"]["start"]["line"], call_line);
    assert_eq!(arr[3]["range"]["start"]["line"], wrap_line);
}

#[test]
fn test_l2_sysroot_atomic_load_definition() {
    let ws = get_or_index_l2_fixture("multi_crate");
    let core_file = fixture_dir("multi_crate").join("crates/core_lib/src/lib.rs");
    let core_uri = path_to_uri(core_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let (load_line, load_col) = pos_in_file(&core_file, "a.load(Ordering::Relaxed)", "load");
    let req = nav_req(&core_uri, load_line, load_col);
    let def_res = handle_definition(&req, Some(ws), &overlay_mgr, &mut conv);
    let decl_res = handle_declaration(&req, Some(ws), &overlay_mgr, &mut conv);
    assert!(!def_res.is_null(), "AtomicUsize::load must resolve via sysroot alias fallback in handle_definition");
    assert_eq!(def_res, decl_res);
    let uri_str = def_res["uri"].as_str().unwrap_or("");
    assert!(uri_str.contains("atomic.rs"), "Definition URI must point to sysroot atomic.rs, got: {}", uri_str);
}
