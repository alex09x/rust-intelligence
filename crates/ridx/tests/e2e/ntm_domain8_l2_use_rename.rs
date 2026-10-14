/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compiler-grounded L2 integration test for `use` import rename operations.
//! Verifies whether renaming a workspace struct symbol edits the corresponding `use` statement in importing modules.

use super::l2_harness::{fixture_dir, get_or_index_l2_fixture, pos_in_file};
use ridx::lsp::editing::rename::handle_rename;
use ridx::lsp::protocol::{path_to_uri, PositionConverter};
use ridx::overlay::OverlayManager;
use serde_json::json;

#[test]
fn test_l2_rename_workspace_symbol_updates_use_statement() {
    let ws = get_or_index_l2_fixture("use_imports");
    let mut conv = PositionConverter::new();
    let overlay_mgr = OverlayManager::new();

    let a_file = fixture_dir("use_imports").join("src/a.rs");
    let b_file = fixture_dir("use_imports").join("src/b.rs");

    let a_file_str = a_file.to_str().expect("valid utf-8 path for a.rs");
    let b_file_str = b_file.to_str().expect("valid utf-8 path for b.rs");

    let (line, col) = pos_in_file(&a_file, "struct Bar", "Bar");
    let a_uri = path_to_uri(a_file_str);
    let b_uri = path_to_uri(b_file_str);

    let loc_refs = ws
        .find_references_at(a_file_str, line + 1, col + 1)
        .expect("must find references at Bar definition");
    let b_refs: Vec<(u32, u32, &str)> = loc_refs
        .references
        .iter()
        .filter(|r| r.file.ends_with("b.rs"))
        .map(|r| (r.span.start_line(), r.span.start_col(), r.file))
        .collect();
    // Verify exactly 3 references in b.rs:
    // 1. `use crate::a::Bar;`
    // 2. `pub fn make_bar() -> Bar`
    // 3. `Bar { value: 42 }`
    assert_eq!(
        b_refs.len(),
        3,
        "Expected exactly 3 references in b.rs, found: {:?}",
        b_refs
    );

    let req = json!({
        "params": {
            "textDocument": { "uri": a_uri },
            "position": { "line": line, "character": col },
            "newName": "NewBar"
        }
    });

    let res = handle_rename(&req, Some(ws), &overlay_mgr, &mut conv)
        .expect("handle_rename on workspace symbol Bar must succeed");

    let changes = res["changes"]
        .as_object()
        .expect("changes object in WorkspaceEdit");

    // Must have changes in both a.rs and b.rs
    assert!(
        changes.contains_key(&b_uri),
        "changes must contain edits for importing module b.rs, but got: {:?}",
        changes.keys().collect::<Vec<_>>()
    );

    let b_edits = changes[&b_uri].as_array().expect("edits array for b.rs");

    // Read b.rs content to verify which lines were modified
    let b_content = std::fs::read_to_string(&b_file).expect("read b.rs");
    let (use_line, use_col) = pos_in_file(&b_file, "use crate::a::Bar;", "Bar");

    let use_edit = b_edits.iter().find(|edit| {
        let start_line = edit["range"]["start"]["line"].as_u64().unwrap_or(u64::MAX) as u32;
        let start_char = edit["range"]["start"]["character"]
            .as_u64()
            .unwrap_or(u64::MAX) as u32;
        start_line == use_line && start_char == use_col
    });

    assert!(
        use_edit.is_some(),
        "rename must produce an edit at the `use` statement in b.rs (line {use_line}, col {use_col}). \
         All edits in b.rs: {:?}",
        b_edits
    );

    // Verify index recorded the reference at the use statement
    assert!(
        b_refs.iter().any(|(line, _, _)| *line == use_line + 1),
        "references in WorkspaceIndex must include use statement in b.rs (line {}). All refs: {:?}",
        use_line + 1,
        b_refs
    );

    // Verify applying edits produces valid modified source code
    let applied_b = ridx_test_support::apply_text_edits(&b_content, b_edits);
    assert!(
        applied_b.contains("use crate::a::NewBar;"),
        "b.rs must contain updated use import after applying edits, but got:\n{applied_b}"
    );
    assert!(
        applied_b.contains("pub fn make_bar() -> NewBar {"),
        "b.rs must contain updated return type after applying edits, but got:\n{applied_b}"
    );
    assert!(
        applied_b.contains("NewBar { value: 42 }"),
        "b.rs must contain updated struct literal after applying edits, but got:\n{applied_b}"
    );
}
