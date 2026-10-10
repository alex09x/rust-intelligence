/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compiler-grounded L2 integration tests for code inside `#[cfg(test)] mod tests`.
//! Validates that goto-definition and hover work seamlessly on items defined
//! inside test modules, which exist in the test unit rather than the library unit.

use super::l2_harness::{fixture_dir, get_or_index_l2_fixture, pos_in_file};
use ridx::lsp::intelligence::hover::handle_hover;
use ridx::lsp::navigation::handle_definition;
use ridx::lsp::protocol::{path_to_uri, PositionConverter};
use ridx::overlay::OverlayManager;
use serde_json::json;

#[test]
fn test_l2_goto_definition_inside_cfg_test_struct() {
    let ws = get_or_index_l2_fixture("shorthand_and_reexports");
    let lib_file = fixture_dir("shorthand_and_reexports").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Query goto-definition on `LocalTestHelper` instantiation site inside `sample_unit_test`
    let (cur_line, cur_col) = pos_in_file(
        &lib_file,
        "let helper = LocalTestHelper { multiplier: 3 };",
        "LocalTestHelper",
    );
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": cur_line, "character": cur_col }
        }
    });

    let res = handle_definition(&req, Some(ws), &overlay_mgr, &mut conv);
    assert!(
        !res.is_null(),
        "Must find definition for struct defined inside #[cfg(test)] module"
    );
    assert_eq!(res["uri"], uri);

    let (def_line, def_col) = pos_in_file(&lib_file, "struct LocalTestHelper {", "LocalTestHelper");
    assert_eq!(res["range"]["start"]["line"], def_line);
    assert_eq!(res["range"]["start"]["character"], def_col);
}

#[test]
fn test_l2_goto_definition_inside_cfg_test_method() {
    let ws = get_or_index_l2_fixture("shorthand_and_reexports");
    let lib_file = fixture_dir("shorthand_and_reexports").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Query goto-definition on method call `compute` inside `sample_unit_test`
    let (cur_line, cur_col) = pos_in_file(&lib_file, "helper.compute(10)", "compute");
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": cur_line, "character": cur_col }
        }
    });

    let res = handle_definition(&req, Some(ws), &overlay_mgr, &mut conv);
    assert!(
        !res.is_null(),
        "Must find definition for method defined inside #[cfg(test)] module"
    );
    assert_eq!(res["uri"], uri);

    let (def_line, def_col) = pos_in_file(
        &lib_file,
        "pub fn compute(&self, val: i32) -> i32 {",
        "compute",
    );
    assert_eq!(res["range"]["start"]["line"], def_line);
    assert_eq!(res["range"]["start"]["character"], def_col);
}

#[test]
fn test_l2_hover_inside_cfg_test_struct() {
    let ws = get_or_index_l2_fixture("shorthand_and_reexports");
    let lib_file = fixture_dir("shorthand_and_reexports").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Query hover on local variable `helper` typed as `LocalTestHelper`
    let (cur_line, cur_col) = pos_in_file(
        &lib_file,
        "let res = helper.compute(10);",
        "helper",
    );
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": cur_line, "character": cur_col }
        }
    });

    let res = handle_hover(&req, Some(ws), &overlay_mgr, &mut conv).unwrap();
    assert!(
        !res.is_null(),
        "Hover on variable inside #[cfg(test)] must return result"
    );
    let val = res["contents"]["value"].as_str().unwrap();
    assert!(
        val.contains("LocalTestHelper"),
        "Hover on helper must contain LocalTestHelper, got: {val}"
    );
}

#[test]
fn test_l2_hover_inside_cfg_test_method() {
    let ws = get_or_index_l2_fixture("shorthand_and_reexports");
    let lib_file = fixture_dir("shorthand_and_reexports").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Query hover on method call `compute`
    let (cur_line, cur_col) = pos_in_file(&lib_file, "helper.compute(10)", "compute");
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": cur_line, "character": cur_col }
        }
    });

    let res = handle_hover(&req, Some(ws), &overlay_mgr, &mut conv).unwrap();
    assert!(
        !res.is_null(),
        "Hover on method inside #[cfg(test)] must return result"
    );
    let val = res["contents"]["value"].as_str().unwrap();
    assert!(
        val.contains("compute") && val.contains("i32"),
        "Hover on compute() must show signature with compute and i32, got: {val}"
    );
}
