/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 3: Compiler-grounded staleness tests (Cases 9, 10, 11), index reloads,
//! and definition fallback chains on real L2 indexed workspaces.

use super::l2_harness::{fixture_dir, get_or_index_l2_fixture, pos_in_str};
use ridx::lsp::navigation::handle_definition;
use ridx::lsp::navigation::references::handle_references;
use ridx::lsp::protocol::{path_to_uri, PositionConverter};
use ridx::overlay::OverlayManager;
use serde_json::json;
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

struct CleanupDir(PathBuf);
impl Drop for CleanupDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn test_case_9_line_shift_definition_and_references() {
    let ws = get_or_index_l2_fixture("traits_and_generics");
    let lib_file = fixture_dir("traits_and_generics").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let original_text = fs::read_to_string(&lib_file).unwrap();
    // Prepend 10 comment lines to shift all subsequent lines down
    let prefix = "// Shift line 1\n// Shift line 2\n// Shift line 3\n// Shift line 4\n// Shift line 5\n\
                  // Shift line 6\n// Shift line 7\n// Shift line 8\n// Shift line 9\n// Shift line 10\n";
    let shifted_text = format!("{}{}", prefix, original_text);

    let file_str = lib_file.to_str().unwrap();
    overlay_mgr.open_with_provenance(&uri, file_str, 2, shifted_text.clone(), false);
    // Baseline is compiled original_text
    overlay_mgr
        .get_mut(file_str)
        .unwrap()
        .advance_compiled_baseline(original_text.clone());

    // 1. Definition query on shifted `Robot` in `call_concrete(r: &Robot)`
    let (cur_line, cur_col) = pos_in_str(&shifted_text, "call_concrete(r: &Robot)", "Robot");
    let def_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": cur_line, "character": cur_col }
        }
    });

    let def_res = handle_definition(&def_req, Some(ws), &overlay_mgr, &mut conv);
    assert!(
        !def_res.is_null(),
        "Must resolve definition despite line shift"
    );
    assert_eq!(def_res["uri"], uri);

    // Target range must point to shifted `pub struct Robot;` line in live buffer!
    let (expected_line, expected_col) = pos_in_str(&shifted_text, "pub struct Robot;", "Robot");
    assert_eq!(def_res["range"]["start"]["line"], expected_line);
    assert_eq!(def_res["range"]["start"]["character"], expected_col);

    // 2. References query on shifted `Robot`
    let ref_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": cur_line, "character": cur_col },
            "context": { "includeDeclaration": true }
        }
    });

    let ref_res = handle_references(&ref_req, Some(ws), &overlay_mgr, &mut conv);
    let refs = ref_res.as_array().expect("references array");
    assert!(!refs.is_empty(), "Must find references on shifted line");

    // All reference ranges in the overlay file must be shifted to match live buffer positions
    let actual_set: BTreeSet<(u64, u64)> = refs
        .iter()
        .map(|r| {
            (
                r["range"]["start"]["line"].as_u64().unwrap(),
                r["range"]["start"]["character"].as_u64().unwrap(),
            )
        })
        .collect();

    let expected_patterns = [
        ("pub struct Robot;", "Robot"),
        ("impl Greeter for Robot {", "Robot"),
        ("pub fn call_concrete(r: &Robot)", "Robot"),
        ("pub fn call_default(r: &Robot)", "Robot"),
    ];
    let expected_set: BTreeSet<(u64, u64)> = expected_patterns
        .iter()
        .map(|(line_pattern, target)| {
            let (l, c) = pos_in_str(&shifted_text, line_pattern, target);
            (l as u64, c as u64)
        })
        .collect();

    assert_eq!(
        actual_set, expected_set,
        "All references must be shifted to match live buffer positions"
    );
}

#[test]
fn test_case_10_stale_rename_overlay_resolution() {
    let ws = get_or_index_l2_fixture("traits_and_generics");
    let lib_file = fixture_dir("traits_and_generics").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let original_text = fs::read_to_string(&lib_file).unwrap();
    // Rename call_default to invoke_default_greeting in live overlay
    let modified_text = original_text.replace(
        "pub fn call_default(r: &Robot) -> &'static str {",
        "pub fn invoke_default_greeting(r: &Robot) -> &'static str {",
    );

    let file_str = lib_file.to_str().unwrap();
    overlay_mgr.open_with_provenance(&uri, file_str, 2, modified_text.clone(), false);
    overlay_mgr
        .get_mut(file_str)
        .unwrap()
        .advance_compiled_baseline(original_text);

    // Query definition of new name `invoke_default_greeting`
    let (cur_line, cur_col) = pos_in_str(
        &modified_text,
        "pub fn invoke_default_greeting",
        "invoke_default_greeting",
    );
    let def_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": cur_line, "character": cur_col }
        }
    });

    let def_res = handle_definition(&def_req, Some(ws), &overlay_mgr, &mut conv);
    assert!(
        !def_res.is_null(),
        "Overlay must resolve freshly renamed symbol locally"
    );
    assert_eq!(def_res["range"]["start"]["line"], cur_line);
    assert_eq!(def_res["range"]["start"]["character"], cur_col);
}

#[test]
fn test_case_11_new_symbol_in_live_buffer() {
    let ws = get_or_index_l2_fixture("shorthand_and_reexports");
    let lib_file = fixture_dir("shorthand_and_reexports").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let original_text = fs::read_to_string(&lib_file).unwrap();
    // Add a brand new function to the end of lib.rs
    let new_fn = "\npub fn new_helper_calculator(val: i32) -> i32 {\n    val * 100\n}\n";
    let modified_text = format!("{}{}", original_text, new_fn);

    let file_str = lib_file.to_str().unwrap();
    overlay_mgr.open_with_provenance(&uri, file_str, 2, modified_text.clone(), false);
    overlay_mgr
        .get_mut(file_str)
        .unwrap()
        .advance_compiled_baseline(original_text);

    let (cur_line, cur_col) = pos_in_str(
        &modified_text,
        "pub fn new_helper_calculator",
        "new_helper_calculator",
    );
    let def_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": cur_line, "character": cur_col }
        }
    });

    let def_res = handle_definition(&def_req, Some(ws), &overlay_mgr, &mut conv);
    assert!(
        !def_res.is_null(),
        "Must resolve definition of newly added symbol in live overlay"
    );
    assert_eq!(def_res["range"]["start"]["line"], cur_line);
    assert_eq!(def_res["range"]["start"]["character"], cur_col);
}

#[test]
fn test_staleness_index_reload_resyncs_overlay() {
    let _ws = get_or_index_l2_fixture("shorthand_and_reexports");
    let lib_file = fixture_dir("shorthand_and_reexports").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let mut overlay_mgr = OverlayManager::new();

    let base_text = fs::read_to_string(&lib_file).unwrap();
    let edited_text = format!("// unindexed change\n{}", base_text);
    let file_str = lib_file.to_str().unwrap();

    overlay_mgr.open_with_provenance(&uri, file_str, 1, edited_text.clone(), false);
    let ov = overlay_mgr.get(file_str).unwrap();
    assert!(ov.is_dirty());

    // Advance compiled baseline to match edited text (simulating post-build index publication)
    overlay_mgr
        .get_mut(file_str)
        .unwrap()
        .advance_compiled_baseline(edited_text.clone());
    let ov_after = overlay_mgr.get(file_str).unwrap();
    assert!(
        !ov_after.is_dirty(),
        "Overlay should be clean after baseline advance"
    );
    assert_eq!(ov_after.map_line_to_compiled(1), Some(1));
}

#[test]
fn test_staleness_query_on_deleted_file() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_del_test_{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let _guard = CleanupDir(temp_dir.clone());

    let file_path = temp_dir.join("deleted_src.rs");
    let code = "pub fn preserved_in_overlay() -> bool { true }\n";
    fs::write(&file_path, code).unwrap();

    let uri = path_to_uri(file_path.to_str().unwrap());
    let file_str = file_path.to_str().unwrap();
    let mut overlay_mgr = OverlayManager::new();

    overlay_mgr.open_with_provenance(&uri, file_str, 1, code.to_string(), true);
    assert_eq!(overlay_mgr.open_count(), 1);

    // Delete file from disk
    fs::remove_file(&file_path).unwrap();
    assert!(!file_path.exists());

    // In-memory overlay continues serving queries
    let ov = overlay_mgr.get(file_str).unwrap();
    assert_eq!(ov.symbols.len(), 1);
    assert_eq!(ov.symbols[0].name, "preserved_in_overlay");

    // didClose evicts
    overlay_mgr.close_document(file_str);
    assert_eq!(overlay_mgr.open_count(), 0);
}

#[test]
fn test_staleness_definition_in_modified_hunk_abstains() {
    let ws = get_or_index_l2_fixture("traits_and_generics");
    let lib_file = fixture_dir("traits_and_generics").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let original_text = fs::read_to_string(&lib_file).unwrap();
    // Replace the body of call_concrete with completely different tokens
    let modified_text =
        original_text.replace("r.greet()", "42 * 999 /* completely modified hunk */");

    let file_str = lib_file.to_str().unwrap();
    overlay_mgr.open_with_provenance(&uri, file_str, 2, modified_text.clone(), false);
    overlay_mgr
        .get_mut(file_str)
        .unwrap()
        .advance_compiled_baseline(original_text);

    // Query on line inside the modified hunk
    let (cur_line, cur_col) = pos_in_str(&modified_text, "42 * 999", "999");
    let def_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": cur_line, "character": cur_col }
        }
    });

    let def_res = handle_definition(&def_req, Some(ws), &overlay_mgr, &mut conv);
    assert!(
        def_res.is_null(),
        "Must abstain rather than return stale compiled definition inside modified hunk"
    );
}

#[test]
fn test_case_9_line_shift_after_did_save_without_reindex() {
    let ws = get_or_index_l2_fixture("traits_and_generics");
    let lib_file = fixture_dir("traits_and_generics").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let original_text = fs::read_to_string(&lib_file).unwrap();
    let prefix = "// Line 1\n// Line 2\n// Line 3\n// Line 4\n// Line 5\n";
    let shifted_text = format!("{}{}", prefix, original_text);

    let file_str = lib_file.to_str().unwrap();
    overlay_mgr.open_with_provenance(&uri, file_str, 2, shifted_text.clone(), false);
    overlay_mgr
        .get_mut(file_str)
        .unwrap()
        .advance_compiled_baseline(original_text);

    // didSave occurs: saves document to disk without re-indexing
    overlay_mgr.save_document(file_str);

    assert!(
        overlay_mgr.get(file_str).unwrap().is_dirty(),
        "Buffer must remain dirty relative to index compiled_text after didSave without reindex"
    );

    // Definition query on shifted `Robot` in `call_concrete(r: &Robot)`
    let (cur_line, cur_col) = pos_in_str(&shifted_text, "call_concrete(r: &Robot)", "Robot");
    let def_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": cur_line, "character": cur_col }
        }
    });

    let def_res = handle_definition(&def_req, Some(ws), &overlay_mgr, &mut conv);
    assert!(
        !def_res.is_null(),
        "Must resolve definition after save without reindex"
    );

    let (expected_line, expected_col) = pos_in_str(&shifted_text, "pub struct Robot;", "Robot");
    assert_eq!(def_res["range"]["start"]["line"], expected_line);
    assert_eq!(def_res["range"]["start"]["character"], expected_col);
}
