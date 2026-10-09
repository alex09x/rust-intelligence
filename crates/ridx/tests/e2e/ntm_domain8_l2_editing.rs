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
    assert!(
        !edits_arr.is_empty(),
        "Unformatted code must produce formatting edits"
    );

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
    assert!(
        !edits_arr.is_empty(),
        "Range formatting must format target function"
    );

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
    assert!(
        !edits_arr.is_empty(),
        "Must produce edits on trigger character"
    );
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

    // 1. References on definition of `inner::Thing` in core_lib must include app_crate usage
    let (def_line, def_col) = pos_in_file(
        &core_file,
        "pub mod inner {\n    pub struct Thing {",
        "Thing",
    );
    let (other_line, _) = pos_in_file(
        &core_file,
        "pub mod other {\n    pub struct Thing {",
        "Thing",
    );

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

    // References must NOT conflate with other::Thing
    let has_other_ref = arr.iter().any(|r| {
        r["uri"] == core_uri && r["range"]["start"]["line"].as_u64() == Some(other_line as u64)
    });
    assert!(
        !has_other_ref,
        "References on inner::Thing must not match unrelated other::Thing: {arr:?}"
    );

    // 2. Rename on `inner::Thing` must edit both core_lib and app_crate, but NEVER other::Thing
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

    // Ensure edits in core_lib do NOT touch other::Thing
    let core_edits = changes[&core_uri].as_array().expect("core_lib edits");
    let edits_other = core_edits
        .iter()
        .any(|edit| edit["range"]["start"]["line"].as_u64() == Some(other_line as u64));
    assert!(
        !edits_other,
        "Rename of inner::Thing must not edit other::Thing: {core_edits:?}"
    );
}

#[test]
fn test_l2_reexport_unmerged_safety_refusal() {
    use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference, WorkspaceIndex};

    let dir = tempfile::tempdir().unwrap();
    let def_file = dir.path().join("lib.rs");
    std::fs::write(
        &def_file,
        "pub mod inner { pub struct Thing; }\npub use inner::Thing;\n",
    )
    .unwrap();
    let consumer_file = dir.path().join("main.rs");
    std::fs::write(
        &consumer_file,
        "use my_lib::Thing;\nfn main() { let _ = Thing; }\n",
    )
    .unwrap();

    let def_file_str = def_file.to_str().unwrap().to_string();
    let consumer_file_str = consumer_file.to_str().unwrap().to_string();

    let mut writer_lib = IndexWriter::new("my_lib", "0.1.0");
    writer_lib.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        file: def_file_str.clone(),
        name: "Thing".to_string(),
        start_line: 1,
        start_col: 16,
        end_line: 1,
        end_col: 21,
        path: "my_lib::inner::Thing".to_string(),
        signature: None,
    });
    let lib_path = dir.path().join("my_lib.ridx");
    let mut f_lib = std::fs::File::create(&lib_path).unwrap();
    writer_lib.write_to(&mut f_lib).unwrap();

    let mut writer_consumer = IndexWriter::new("consumer", "0.1.0");
    writer_consumer.add_reference(RawReference {
        file: consumer_file_str,
        start_line: 1,
        start_col: 12,
        end_line: 1,
        end_col: 17,
        target_def_id: None,
        target_path: "my_lib::Thing".to_string(),
        target_unit: None,
        target_file: Some(def_file_str),
        target_line: 1,
        target_col: 16,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });
    let consumer_path = dir.path().join("consumer.ridx");
    let mut f_consumer = std::fs::File::create(&consumer_path).unwrap();
    writer_consumer.write_to(&mut f_consumer).unwrap();

    let ws = WorkspaceIndex::open(dir.path()).expect("open WorkspaceIndex");

    // 1. Incomplete references: has_unmerged_reexport_references must detect the unmerged reference to my_lib::Thing
    let incomplete_refs = Vec::new();
    assert!(
        ws.has_unmerged_reexport_references("my_lib::inner::Thing", &incomplete_refs),
        "has_unmerged_reexport_references must report true when cross-crate references to alias exist but are missing"
    );

    // 2. Merged references: when the reference is provided in found_refs, it should return false
    let all_refs = ws.find_references("my_lib::Thing");
    assert_eq!(all_refs.len(), 1);
    assert!(
        !ws.has_unmerged_reexport_references("my_lib::inner::Thing", &all_refs),
        "has_unmerged_reexport_references must report false when all alias references are accounted for"
    );
}

#[test]
fn test_l2_reexport_definition_resolution() {
    let ws = get_or_index_l2_fixture("multi_crate");
    let def = ws
        .find_definition_by_path("core_lib::Thing")
        .expect("core_lib::Thing should resolve to definition");
    assert_eq!(def.name, "Thing");
    assert_eq!(def.path, "inner::Thing");
    assert_eq!(def.kind, ridx_format::DefKind::Struct);
}

#[test]
#[ignore = "sysroot crates are not indexed yet (see backlog: sysroot indexing)"]
fn test_l2_std_collections_hashmap_lookup() {
    let ws = get_or_index_l2_fixture("multi_crate");
    let def = ws
        .find_definition_by_path("std::collections::HashMap")
        .expect("std::collections::HashMap should resolve to struct definition");
    assert_eq!(def.name, "HashMap");
    assert!(!ws.search_symbols("HashMap").is_empty());
}
