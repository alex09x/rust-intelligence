/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compiler-grounded L2 integration test cases (Cases 1–8) executed on real
//! `ridx-driver` indexed workspaces.

use super::l2_harness::{fixture_dir, get_or_index_l2_fixture, pos_in_file};
use ridx::lsp::editing::rename::{handle_prepare_rename, handle_rename};
use ridx::lsp::navigation::handle_definition;
use ridx::lsp::navigation::references::handle_references;
use ridx::lsp::protocol::{path_to_uri, PositionConverter};
use ridx::overlay::OverlayManager;
use serde_json::json;

#[test]
#[ignore = "assoc item constraint: known gap in ridx-driver (visit_assoc_item_constraint not yet implemented), see backlog"]
fn test_case_1_associated_type_navigation() {
    let ws = get_or_index_l2_fixture("traits_and_generics");
    let lib_file = fixture_dir("traits_and_generics").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Cursor on `Item` in `Container<Item = i32>`
    let (cur_line, cur_col) = pos_in_file(&lib_file, "Container<Item = i32>", "Item");
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": cur_line, "character": cur_col }
        }
    });

    let res = handle_definition(&req, Some(ws), &overlay_mgr, &mut conv);
    assert!(
        !res.is_null(),
        "Must find definition for associated item constraint"
    );
    assert_eq!(res["uri"], uri);

    let (def_line, def_col) = pos_in_file(&lib_file, "type Item;", "Item");
    assert_eq!(res["range"]["start"]["line"], def_line);
    assert_eq!(res["range"]["start"]["character"], def_col);
    assert_eq!(res["range"]["end"]["character"], def_col + 4);
}

#[test]
fn test_case_2_trait_method_concrete_vs_generic() {
    let ws = get_or_index_l2_fixture("traits_and_generics");
    let lib_file = fixture_dir("traits_and_generics").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Concrete method dispatch: r.greet()
    let (cur_line, cur_col) = pos_in_file(&lib_file, "r.greet()", "greet");
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": cur_line, "character": cur_col }
        }
    });
    let res = handle_definition(&req, Some(ws), &overlay_mgr, &mut conv);
    assert!(
        !res.is_null(),
        "Must resolve concrete trait method dispatch"
    );
    assert_eq!(res["uri"], uri);

    let (def_line, def_col) =
        pos_in_file(&lib_file, "impl Greeter for Robot {\n    fn greet", "greet");
    assert_eq!(res["range"]["start"]["line"], def_line);
    assert_eq!(res["range"]["start"]["character"], def_col);
    assert_eq!(res["range"]["end"]["character"], def_col + 5);

    // Default method dispatch: r.default_greet()
    let (def_cur_line, def_cur_col) = pos_in_file(&lib_file, "r.default_greet()", "default_greet");
    let def_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": def_cur_line, "character": def_cur_col }
        }
    });
    let def_res = handle_definition(&def_req, Some(ws), &overlay_mgr, &mut conv);
    assert!(!def_res.is_null(), "Must resolve trait default method");
    assert_eq!(def_res["uri"], uri);
    let (trait_def_line, trait_def_col) =
        pos_in_file(&lib_file, "fn default_greet(&self)", "default_greet");
    assert_eq!(def_res["range"]["start"]["line"], trait_def_line);
    assert_eq!(def_res["range"]["start"]["character"], trait_def_col);
}

#[test]
fn test_case_3_two_versions_of_same_crate() {
    let ws = get_or_index_l2_fixture("multi_crate");
    let app_file = fixture_dir("multi_crate").join("crates/app_crate/src/lib.rs");
    let app_uri = path_to_uri(app_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Call 1: calc_v1::calculate()
    let (cur_v1_line, cur_v1_col) = pos_in_file(&app_file, "calc_v1::calculate()", "calculate");
    let req_v1 = json!({
        "params": {
            "textDocument": { "uri": &app_uri },
            "position": { "line": cur_v1_line, "character": cur_v1_col }
        }
    });
    let res_v1 = handle_definition(&req_v1, Some(ws), &overlay_mgr, &mut conv);
    assert!(
        !res_v1.is_null(),
        "Definition for calc_v1 call must be found"
    );
    let uri_v1_str = res_v1["uri"].as_str().unwrap();
    assert!(
        uri_v1_str.contains("calc_v1/src/lib.rs"),
        "Expected calc_v1, got {uri_v1_str}"
    );

    // Call 2: calc_v2::calculate()
    let (cur_v2_line, cur_v2_col) = pos_in_file(&app_file, "calc_v2::calculate()", "calculate");
    let req_v2 = json!({
        "params": {
            "textDocument": { "uri": &app_uri },
            "position": { "line": cur_v2_line, "character": cur_v2_col }
        }
    });
    let res_v2 = handle_definition(&req_v2, Some(ws), &overlay_mgr, &mut conv);
    assert!(
        !res_v2.is_null(),
        "Definition for calc_v2 call must be found"
    );
    let uri_v2_str = res_v2["uri"].as_str().unwrap();
    assert!(
        uri_v2_str.contains("calc_v2/src/lib.rs"),
        "Expected calc_v2, got {uri_v2_str}"
    );
    assert_ne!(
        uri_v1_str, uri_v2_str,
        "Two versions must resolve to distinct paths"
    );
}

#[test]
fn test_case_4_exact_ident_span_and_raw_idents() {
    let ws = get_or_index_l2_fixture("shorthand_and_reexports");
    let lib_file = fixture_dir("shorthand_and_reexports").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Raw ident definition: `pub fn r#match(...)`
    let (cur_line, cur_col) = pos_in_file(&lib_file, "r#match(42)", "match");
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": cur_line, "character": cur_col }
        }
    });
    let res = handle_definition(&req, Some(ws), &overlay_mgr, &mut conv);
    assert!(!res.is_null());
    assert_eq!(res["uri"], uri);

    let (def_line, def_col) = pos_in_file(&lib_file, "pub fn r#match", "r#match");
    assert_eq!(res["range"]["start"]["line"], def_line);
    assert_eq!(res["range"]["start"]["character"], def_col);
    assert_eq!(res["range"]["end"]["character"], def_col + 7);
}

#[test]
fn test_case_5_macros_and_derives_navigation() {
    let ws = get_or_index_l2_fixture("macros_and_derives");
    let lib_file = fixture_dir("macros_and_derives").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Call to `triple(10)` generated by macro
    let (cur_line, cur_col) = pos_in_file(&lib_file, "triple(10)", "triple");
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": cur_line, "character": cur_col }
        }
    });
    let res = handle_definition(&req, Some(ws), &overlay_mgr, &mut conv);
    assert!(!res.is_null());
    assert_eq!(res["uri"], uri);
    let (def_line, def_col) = pos_in_file(&lib_file, "make_multiplier!(triple, 3);", "triple");
    assert_eq!(res["range"]["start"]["line"], def_line);
    assert_eq!(res["range"]["start"]["character"], def_col);
    assert_eq!(res["range"]["end"]["character"], def_col + 6);

    // Call to included function `included_helper()`
    let (inc_line, inc_col) = pos_in_file(&lib_file, "included_helper()", "included_helper");
    let inc_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": inc_line, "character": inc_col }
        }
    });
    let inc_res = handle_definition(&inc_req, Some(ws), &overlay_mgr, &mut conv);
    assert!(!inc_res.is_null());
    assert!(inc_res["uri"].as_str().unwrap().ends_with("included.rs"));
}

#[test]
fn test_case_6_field_shorthand_and_destructuring() {
    let ws = get_or_index_l2_fixture("shorthand_and_reexports");
    let lib_file = fixture_dir("shorthand_and_reexports").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // x shorthand in Point { x, y }
    let (cur_line, cur_col) = pos_in_file(
        &lib_file,
        "pub fn make_point(x: i32, y: i32) -> Point {\n    Point { x, y }",
        "x, y }",
    );
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": cur_line, "character": cur_col }
        }
    });
    let res = handle_definition(&req, Some(ws), &overlay_mgr, &mut conv);
    assert!(!res.is_null());
    assert_eq!(res["uri"], uri);
    let (def_line, def_col) = pos_in_file(&lib_file, "pub fn make_point(x: i32", "x");
    assert_eq!(res["range"]["start"]["line"], def_line);
    assert_eq!(res["range"]["start"]["character"], def_col);
    assert_eq!(res["range"]["end"]["character"], def_col + 1);
}

#[test]
fn test_case_7_aliased_use_and_reexports() {
    let ws = get_or_index_l2_fixture("shorthand_and_reexports");
    let lib_file = fixture_dir("shorthand_and_reexports").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Query aliased re-export in signature: AliasedWidget
    let (cur_line, cur_col) = pos_in_file(
        &lib_file,
        "pub fn invoke_aliased_widget() -> AliasedWidget",
        "AliasedWidget",
    );
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": cur_line, "character": cur_col }
        }
    });
    let res = handle_definition(&req, Some(ws), &overlay_mgr, &mut conv);
    assert!(!res.is_null());
    assert_eq!(res["uri"], uri);
    let (def_line, def_col) = pos_in_file(&lib_file, "pub struct InnerWidget", "InnerWidget");
    assert_eq!(res["range"]["start"]["line"], def_line);
    assert_eq!(res["range"]["start"]["character"], def_col);
    assert_eq!(res["range"]["end"]["character"], def_col + 11);
}

#[test]
fn test_case_8_external_dependency_rename_safety() {
    let ws = get_or_index_l2_fixture("multi_crate");
    let app_file = fixture_dir("multi_crate").join("crates/app_crate/src/lib.rs");
    let app_uri = path_to_uri(app_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Cursor on `external_compute()` in `ext_service::external_compute()` (external dependency)
    let (cur_line, cur_col) = pos_in_file(
        &app_file,
        "ext_service::external_compute()",
        "external_compute",
    );
    let prep_req = json!({
        "params": {
            "textDocument": { "uri": &app_uri },
            "position": { "line": cur_line, "character": cur_col }
        }
    });
    let prep_res = handle_prepare_rename(&prep_req, Some(ws), &overlay_mgr, &mut conv);
    assert!(
        prep_res.is_err(),
        "prepareRename on external dependency must be rejected"
    );
    assert_eq!(prep_res.unwrap_err().code, -32803);

    let ren_req = json!({
        "params": {
            "textDocument": { "uri": &app_uri },
            "position": { "line": cur_line, "character": cur_col },
            "newName": "renamed_compute"
        }
    });
    let ren_res = handle_rename(&ren_req, Some(ws), &overlay_mgr, &mut conv);
    assert!(
        ren_res.is_err(),
        "rename on external dependency must be rejected"
    );
    assert_eq!(ren_res.unwrap_err().code, -32803);

    let original = std::fs::read_to_string(&app_file).unwrap();
    assert!(original.contains("ext_service::external_compute()"));

    let ext_file = fixture_dir("_vendor").join(".cargo/registry/src/ext_service-0.5.0/src/lib.rs");
    let ext_content = std::fs::read_to_string(&ext_file).unwrap();
    assert!(ext_content.contains("pub fn external_compute()"));
}

#[test]
fn test_case_cross_crate_ident_name_isolation() {
    let ws = get_or_index_l2_fixture("multi_crate");
    let app_file = fixture_dir("multi_crate").join("crates/app_crate/src/lib.rs");
    let app_uri = path_to_uri(app_file.to_str().unwrap());
    let core_file = fixture_dir("multi_crate").join("crates/core_lib/src/lib.rs");
    let core_uri = path_to_uri(core_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Find references to `core_lib::CoreService::execute`
    let (core_line, core_col) = pos_in_file(&core_file, "pub fn execute(&self)", "execute");
    let ref_req = json!({
        "params": {
            "textDocument": { "uri": &core_uri },
            "position": { "line": core_line, "character": core_col },
            "context": { "includeDeclaration": true }
        }
    });
    let ref_res = handle_references(&ref_req, Some(ws), &overlay_mgr, &mut conv);
    let arr = ref_res.as_array().expect("references array");

    // References must not include app_crate's local `CoreService::execute`!
    let (app_local_line, _) = pos_in_file(
        &app_file,
        "impl CoreService {\n    pub fn execute",
        "execute",
    );
    let (app_local_call_line, _) = pos_in_file(&app_file, "local_svc.execute()", "execute");

    for r in arr {
        if r["uri"] == app_uri {
            let l = r["range"]["start"]["line"].as_u64().unwrap() as u32;
            assert_ne!(
                l, app_local_line,
                "Must not match local CoreService definition in app_crate"
            );
            assert_ne!(
                l, app_local_call_line,
                "Must not match local_svc.execute() call in app_crate"
            );
        }
    }
}
