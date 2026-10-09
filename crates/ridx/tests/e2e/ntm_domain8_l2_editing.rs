/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compiler-grounded L2 integration tests for Domain 8: Editing & Formatting.
//! Tests formatting, rangeFormatting, onTypeFormatting, and linkedEditingRange.

use super::l2_harness::{fixture_dir, get_or_index_l2_fixture, pos_in_file};
use ridx::lsp::editing::format::{
    find_crate_edition, handle_formatting, handle_on_type_formatting, handle_range_formatting,
};
use ridx::lsp::editing::linked::handle_linked_editing_range;
use ridx::lsp::editing::rename::handle_rename;
use ridx::lsp::navigation::references::handle_references;
use ridx::lsp::protocol::{path_to_uri, PositionConverter};
use ridx::overlay::OverlayManager;
use ridx_test_support::apply_text_edits;
use serde_json::json;

fn is_rustfmt_installed() -> bool {
    std::process::Command::new("rustfmt")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn test_l2_formatting_full_and_idempotence() {
    if !is_rustfmt_installed() {
        return;
    }

    let mut overlay_mgr = OverlayManager::new();
    let file = fixture_dir("traits_and_generics").join("src/lib.rs");
    let uri = path_to_uri(file.to_str().unwrap());

    let unformatted = "pub fn add_unformatted( a:i32,b:i32 )->i32{   a+b  }\n";
    overlay_mgr.open_document(&uri, file.to_str().unwrap(), 1, unformatted.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });

    let edits = handle_formatting(&req, &overlay_mgr);
    let edits_arr = edits.as_array().expect("edits array");
    assert!(!edits_arr.is_empty(), "Unformatted code must produce formatting edits");

    let formatted = apply_text_edits(unformatted, edits_arr);
    assert_ne!(formatted, unformatted);
    assert!(formatted.contains("pub fn add_unformatted(a: i32, b: i32) -> i32 {"));

    // Verify idempotence: formatting formatted code produces 0 edits
    overlay_mgr.open_document(&uri, file.to_str().unwrap(), 2, formatted.clone());
    let second_edits = handle_formatting(&req, &overlay_mgr);
    assert_eq!(
        second_edits,
        json!([]),
        "Formatting already formatted code must be idempotent"
    );
}

#[test]
fn test_l2_range_formatting_scoped() {
    if !is_rustfmt_installed() {
        return;
    }

    let mut overlay_mgr = OverlayManager::new();
    let file = fixture_dir("traits_and_generics").join("src/lib.rs");
    let uri = path_to_uri(file.to_str().unwrap());

    let code = concat!(
        "// line 0: clean\n",
        "fn untouched( a : i32 ) -> i32 { a }\n",
        "\n",
        "fn target( x:i32 , y:i32 )->i32{\n",
        "let z = x*y ;\n",
        "z\n",
        "}\n",
    );
    overlay_mgr.open_document(&uri, file.to_str().unwrap(), 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "range": {
                "start": { "line": 3, "character": 0 },
                "end": { "line": 6, "character": 1 }
            },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });

    let edits = handle_range_formatting(&req, &overlay_mgr);
    let edits_arr = edits.as_array().expect("range edits array");
    assert!(!edits_arr.is_empty(), "Range formatting must format target function");

    let result = apply_text_edits(code, edits_arr);
    assert!(
        result.contains("fn untouched( a : i32 ) -> i32 { a }"),
        "Untouched line outside range must remain unchanged"
    );
    assert!(
        result.contains("fn target(x: i32, y: i32) -> i32 {"),
        "Target function in range must be formatted"
    );
}

#[test]
fn test_l2_on_type_formatting_delegation() {
    if !is_rustfmt_installed() {
        return;
    }

    let mut overlay_mgr = OverlayManager::new();
    let file = fixture_dir("traits_and_generics").join("src/lib.rs");
    let uri = path_to_uri(file.to_str().unwrap());

    let code = "fn test_type(){let a=1;}\n";
    overlay_mgr.open_document(&uri, file.to_str().unwrap(), 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 23 },
            "ch": ";",
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });

    let edits = handle_on_type_formatting(&req, &overlay_mgr);
    assert!(edits.is_array(), "onTypeFormatting returns edit array");
    let edits_arr = edits.as_array().unwrap();
    assert!(!edits_arr.is_empty(), "Must produce edits on trigger character");
}

#[test]
fn test_l2_linked_editing_range_local_bindings() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let file = fixture_dir("traits_and_generics").join("src/lib.rs");
    let uri = path_to_uri(file.to_str().unwrap());

    let code = concat!(
        "fn calculate_total() -> i32 {\n",
        "    let amount = 100;\n",
        "    let tax = amount * 2;\n",
        "    amount + tax\n",
        "}\n",
    );
    overlay_mgr.open_document(&uri, file.to_str().unwrap(), 1, code.to_string());

    // 1. Linked editing on `amount` local binding (line 1, col 8)
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 8 }
        }
    });

    let res = handle_linked_editing_range(&req, &overlay_mgr, &mut conv);
    if !res.is_null() {
        let ranges = res["ranges"].as_array().expect("ranges array");
        assert!(
            ranges.len() >= 2,
            "Linked editing must return all local binding references"
        );
        for r in ranges {
            assert!(r["start"]["line"].as_u64().is_some());
            assert!(r["start"]["character"].as_u64().is_some());
            assert!(r["end"]["line"].as_u64().is_some());
            assert!(r["end"]["character"].as_u64().is_some());
        }
    }

    // 2. Linked editing on non-existent position returns null
    let missing_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 999, "character": 0 }
        }
    });
    let null_res = handle_linked_editing_range(&missing_req, &overlay_mgr, &mut conv);
    assert!(null_res.is_null(), "Non-existent position returns null");
}

#[test]
fn test_l2_edition_resolution() {
    let fixture_lib = fixture_dir("traits_and_generics").join("src/lib.rs");
    let edition = find_crate_edition(fixture_lib.to_str().unwrap());
    assert!(
        matches!(edition.as_str(), "2015" | "2018" | "2021" | "2024"),
        "find_crate_edition must return a valid Rust edition: {edition}"
    );
}

#[test]
fn test_l2_cross_crate_reexport_references_and_rename() {
    let ws = get_or_index_l2_fixture("multi_crate");
    let core_file = fixture_dir("multi_crate").join("crates/core_lib/src/lib.rs");
    let core_uri = path_to_uri(core_file.to_str().unwrap());
    let app_file = fixture_dir("multi_crate").join("crates/app_crate/src/lib.rs");
    let app_uri = path_to_uri(app_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // 1. References on definition of `Thing` in core_lib must include app_crate usage
    let (def_line, def_col) = pos_in_file(&core_file, "pub struct Thing {", "Thing");
    let ref_req = json!({
        "params": {
            "textDocument": { "uri": &core_uri },
            "position": { "line": def_line, "character": def_col },
            "context": { "includeDeclaration": true }
        }
    });
    let ref_res = handle_references(&ref_req, Some(ws), &overlay_mgr, &mut conv);
    let arr = ref_res.as_array().expect("references array");

    let has_core_ref = arr.iter().any(|r| r["uri"] == core_uri);
    let has_app_ref = arr.iter().any(|r| r["uri"] == app_uri);
    assert!(
        has_core_ref,
        "References must include core_lib definition/re-export: {arr:?}"
    );
    assert!(
        has_app_ref,
        "References across pub use must include app_crate usage: {arr:?}"
    );

    // 2. Rename on `Thing` must edit both core_lib and app_crate
    let rename_req = json!({
        "params": {
            "textDocument": { "uri": &core_uri },
            "position": { "line": def_line, "character": def_col },
            "newName": "RenamedThing"
        }
    });
    let rename_res = handle_rename(&rename_req, Some(ws), &overlay_mgr, &mut conv)
        .expect("rename across pub use succeeds");
    let changes = rename_res["changes"].as_object().expect("changes map");
    assert!(
        changes.contains_key(&core_uri),
        "Rename must edit core_lib: {changes:?}"
    );
    assert!(
        changes.contains_key(&app_uri),
        "Rename across pub use must edit app_crate: {changes:?}"
    );
}

#[test]
fn test_l2_reexport_unmerged_safety_refusal() {
    let ws = get_or_index_l2_fixture("multi_crate");
    // Verify that has_unmerged_reexport_references detects missing cross-crate references
    let incomplete_refs = Vec::new();
    assert!(
        ws.has_unmerged_reexport_references("core_lib::inner::Thing", &incomplete_refs),
        "has_unmerged_reexport_references must report true when cross-crate references are missing"
    );
}

