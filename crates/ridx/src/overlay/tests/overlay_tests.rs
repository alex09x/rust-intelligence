/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use crate::overlay::{DocumentOverlay, OverlayManager};
use ridx_format::{normalized_source_hash, DefKind};

#[test]
fn test_multibyte_non_identifier_is_safe() {
    let code = "fn f() { 🚀 }\n".to_string();
    let overlay = DocumentOverlay::new("file:///test/emoji.rs", "src/emoji.rs", 1, code);
    assert!(!overlay.symbols.is_empty());
    let def = overlay.find_definition_at(1, 4).expect("should resolve f");
    assert_eq!(def.name, "f");
}

#[test]
fn test_shadow_initializer_sees_previous_binding() {
    let code = "fn test() {\n    let x = 1;\n    let x = x + 1;\n    x;\n}\n".to_string();
    let overlay = DocumentOverlay::new("file:///test/shadow_init.rs", "src/shadow_init.rs", 1, code);
    // Cursor on `x` in `x + 1` at line 3, col 13
    let def = overlay.find_definition_at(3, 13).expect("should resolve to outer x");
    assert_eq!(def.name, "x");
    assert_eq!(def.start_line, 2);

    // Cursor on `x;` on line 4, col 5 -> resolves to second `let x = x + 1` on line 3
    let def2 = overlay.find_definition_at(4, 5).expect("should resolve to inner x");
    assert_eq!(def2.name, "x");
    assert_eq!(def2.start_line, 3);
}

#[test]
fn test_qualified_name_does_not_resolve_lexically() {
    let code = "fn f() {}\nfn use_it() { external::f(); }\n".to_string();
    let overlay = DocumentOverlay::new("file:///test/qual.rs", "src/qual.rs", 1, code);
    // Querying external::f on line 2, col 25 (the `f` in `external::f`) should abstain (None)
    let def = overlay.find_definition_at(2, 25);
    assert!(def.is_none(), "overlay must abstain for qualified path segment external::f");
}

#[test]
fn test_nested_initializer_keeps_outer_binding() {
    let code = "fn test() {\n    let x = 1;\n    let x = { let t = 2; x };\n    x;\n}\n".to_string();
    let overlay = DocumentOverlay::new("file:///test/nested_init.rs", "src/nested_init.rs", 1, code);
    // Cursor on `x` inside block `{ let t = 2; x }` on line 3, col 27 -> resolves to line 2 (outer x)
    let def = overlay.find_definition_at(3, 27).expect("should resolve to outer x");
    assert_eq!(def.name, "x");
    assert_eq!(def.start_line, 2);

    // Cursor on `x;` on line 4, col 5 -> resolves to line 3 (inner x)
    let def2 = overlay.find_definition_at(4, 5).expect("should resolve to inner x");
    assert_eq!(def2.name, "x");
    assert_eq!(def2.start_line, 3);
}

#[test]
fn test_multiline_qualified_path_abstains() {
    let code = "fn f() {}\nfn run() { external::\n    f();\n}\n".to_string();
    let overlay = DocumentOverlay::new("file:///test/multiline_qual.rs", "src/multiline_qual.rs", 1, code);
    // Querying `f` on line 3, col 5 should abstain (None) because of `external::` on previous line
    let def = overlay.find_definition_at(3, 5);
    assert!(def.is_none(), "overlay must abstain for multiline qualified path external::\\n    f()");
}

#[test]
fn test_nested_comment_qualification_abstains() {
    let code = "fn f() {}\nfn run() { external::/* outer /* inner */ outer */f();\n}\n".to_string();
    let overlay = DocumentOverlay::new("file:///test/nested_comment.rs", "src/nested_comment.rs", 1, code);
    // Querying `f` on line 2, col 51 should abstain (None) because of `external::...` preceding it
    let def = overlay.find_definition_at(2, 51);
    assert!(def.is_none(), "overlay must abstain for qualified path with nested comments external::/* outer /* inner */ outer */f()");
}

#[test]
fn test_save_does_not_advance_compiled_baseline() {
    let initial_code = "fn a() {}\nfn b() {}\n".to_string();
    let mut overlay = DocumentOverlay::new("file:///test/save.rs", "src/save.rs", 1, initial_code);
    // User inserts a new line at top
    let edited_code = "// new line\nfn a() {}\nfn b() {}\n".to_string();
    overlay.update(2, edited_code.clone());
    // In edited buffer, line 3 (fn b) maps to compiled line 2
    assert_eq!(overlay.map_line_to_compiled(3), Some(2));

    // Now simulate didSave with the edited text hash
    let _hash = normalized_source_hash(edited_code.as_bytes());
    overlay.disk_text = Some(edited_code.clone());
    // Since no compilation occurred, compiled baseline is still initial_code!
    assert_eq!(overlay.map_line_to_compiled(3), Some(2));
    assert!(overlay.is_dirty());
}

#[test]
fn test_overlay_local_variables_and_params() {
    let code = "fn compute(input: i32) {\n    let result = input * 2;\n    result\n}\n".to_string();
    let overlay = DocumentOverlay::new("file:///test/main.rs", "src/main.rs", 1, code);

    // 1. Cursor on `result` in expression on line 3, col 5
    let def = overlay.find_definition_at(3, 5).expect("should find local variable 'result'");
    assert_eq!(def.name, "result");
    assert_eq!(def.start_line, 2);
    assert_eq!(def.kind, DefKind::Local);

    // 2. Cursor on `input` in expression on line 2, col 19
    let def_param = overlay.find_definition_at(2, 19).expect("should find parameter 'input'");
    assert_eq!(def_param.name, "input");
    assert_eq!(def_param.start_line, 1);
    assert_eq!(def_param.kind, DefKind::Local);

    // 3. Cursor on function name `compute` at line 1, col 4
    let def_fn = overlay.find_definition_at(1, 4).expect("should find function 'compute'");
    assert_eq!(def_fn.name, "compute");
    assert_eq!(def_fn.kind, DefKind::Fn);
    assert_eq!(def_fn.start_line, 1);
}

#[test]
fn test_unicode_binding_does_not_panic() {
    let code = "fn f() {\n    let grüßen = 1;\n    let world = grüßen;\n}\n".to_string();
    let overlay = DocumentOverlay::new("file:///test/unicode.rs", "src/unicode.rs", 1, code);

    assert!(!overlay.symbols.is_empty());
    let def = overlay.find_definition_at(3, 18).expect("should resolve 'grüßen'");
    assert_eq!(def.name, "grüßen");
    assert_eq!(def.start_line, 2);

    let world = overlay.find_definition_at(3, 10).expect("should resolve 'world'");
    assert_eq!(world.name, "world");
    assert_eq!(world.start_line, 3);
}

#[test]
fn test_function_parameter_scope_isolation() {
    let code = "fn a(x: i32) {}\nfn b(x: i32) { x; }\n".to_string();
    let overlay = DocumentOverlay::new("file:///test/scopes.rs", "src/scopes.rs", 1, code);

    // Cursor on `x;` inside b: line 2, col 16
    let def = overlay.find_definition_at(2, 16).expect("should find x in b");
    assert_eq!(def.name, "x");
    assert_eq!(def.start_line, 2);
}

#[test]
fn test_lexical_binding_shadowing_and_visibility() {
    let code = "fn test() {\n    let x = 1;\n    let x = 2;\n    x;\n}\n".to_string();
    let overlay = DocumentOverlay::new("file:///test/shadow.rs", "src/shadow.rs", 1, code);

    // Cursor on `x;` on line 4, col 5 -> should resolve to second `let x = 2` on line 3
    let def = overlay.find_definition_at(4, 5).expect("should resolve nearest shadowed binding");
    assert_eq!(def.name, "x");
    assert_eq!(def.start_line, 3);
}

#[test]
fn test_overlay_edit_mapping() {
    let saved_text = "fn foo() {\n    call_a();\n    call_b();\n}\n".to_string();
    let mut overlay = DocumentOverlay::new("file:///test/edit.rs", "src/edit.rs", 1, saved_text.clone());
    overlay.disk_text = Some(saved_text);

    assert_eq!(overlay.map_line_to_compiled(2), Some(2));

    // Insert a new line at line 2:
    let edited = "fn foo() {\n    // new comment\n    call_a();\n    call_b();\n}\n".to_string();
    overlay.update(2, edited);

    // Line 1 (unchanged prefix) maps to line 1
    assert_eq!(overlay.map_line_to_compiled(1), Some(1));
    // Line 2 (newly inserted comment) is modified region -> None
    assert_eq!(overlay.map_line_to_compiled(2), None);
    // Line 3 (`call_a()`) is in unchanged suffix -> maps to line 2 in compiled snapshot!
    assert_eq!(overlay.map_line_to_compiled(3), Some(2));
    // Line 4 (`call_b()`) -> maps to line 3 in compiled snapshot!
    assert_eq!(overlay.map_line_to_compiled(4), Some(3));
}

#[test]
fn test_overlay_position_conversion_with_unicode() {
    let code = "let grüßen = 42;\n".to_string();
    let overlay = DocumentOverlay::new("file:///test/conv.rs", "src/conv.rs", 1, code);

    // 'let ' = 4 chars / 4 utf-16
    // 'grüßen' = 6 Unicode chars (each 1 UTF-16 code unit)
    // utf-16 col 4 -> 1-based char col 5 (the start of 'grüßen')
    let char_col = overlay.utf16_col_to_char_col(1, 4);
    assert_eq!(char_col, 5);

    let utf16_col = overlay.char_col_to_utf16_col(1, 5);
    assert_eq!(utf16_col, 4);
}

#[test]
fn test_overlay_incomplete_syntax_tolerance() {
    let broken = "fn unclosed(val: &str) {\n    let partial = val.\n".to_string();
    let overlay = DocumentOverlay::new("file:///test/broken.rs", "src/broken.rs", 1, broken);

    assert!(!overlay.symbols.is_empty());
    let fn_sym = overlay.symbols.iter().find(|s| s.name == "unclosed").expect("should extract function name");
    assert_eq!(fn_sym.kind, DefKind::Fn);

    let let_sym = overlay.symbols.iter().find(|s| s.name == "partial").expect("should extract incomplete let binding");
    assert_eq!(let_sym.start_line, 2);
}

#[test]
fn test_overlay_manager_reconciliation() {
    let mut mgr = OverlayManager::new();
    let uri = "file:///test/live.rs";
    let file = "src/live.rs";
    let text1 = "fn foo() {}\n".to_string();

    mgr.open_or_update(uri, file, 1, text1.clone());
    assert_eq!(mgr.open_count(), 1);

    let hash1 = normalized_source_hash(text1.as_bytes());
    mgr.reconcile_with_disk(file, hash1);
    assert!(mgr.get(file).unwrap().is_synced_with_disk());

    // User keeps typing while cargo check runs
    let text2 = "fn foo() { let x = 42; }\n".to_string();
    mgr.open_or_update(uri, file, 2, text2);
    assert!(!mgr.get(file).unwrap().is_synced_with_disk());

    mgr.close(file);
    assert_eq!(mgr.open_count(), 0);
}

#[test]
fn test_unterminated_unicode_block_comment_is_safe() {
    let broken = "fn f() { /* 🚀".to_string();
    let overlay = DocumentOverlay::new("file:///test/broken.rs", "src/broken.rs", 1, broken);
    assert_eq!(overlay.symbols.len(), 1);
    assert_eq!(overlay.symbols[0].name, "f");

    let broken_let = "fn f() {\n    let x = /* 🚀\n}\n".to_string();
    let overlay_let = DocumentOverlay::new("file:///test/broken_let.rs", "src/broken_let.rs", 1, broken_let);
    assert_eq!(overlay_let.symbols.len(), 2);
    assert_eq!(overlay_let.symbols[0].name, "f");
    assert_eq!(overlay_let.symbols[1].name, "x");
}

#[test]
fn test_incremental_change_single_line() {
    let initial = "fn main() {\n    let old_name = 100;\n    let y = old_name;\n}\n".to_string();
    let mut overlay = DocumentOverlay::new("file:///test/inc.rs", "src/inc.rs", 1, initial);

    // Verify initial resolution
    let def = overlay.find_definition_at(3, 15).expect("resolve old_name");
    assert_eq!(def.name, "old_name");

    // Incremental change 1: replace "old_name" with "new_var" on line 2 (0-based line 1, chars 8..16)
    let change1 = serde_json::json!({
        "range": {
            "start": { "line": 1, "character": 8 },
            "end": { "line": 1, "character": 16 }
        },
        "text": "new_var"
    });

    // Incremental change 2: replace "old_name" with "new_var" on line 3 (0-based line 2, chars 12..20)
    let change2 = serde_json::json!({
        "range": {
            "start": { "line": 2, "character": 12 },
            "end": { "line": 2, "character": 20 }
        },
        "text": "new_var"
    });

    overlay.apply_content_changes(2, &[change1, change2]);
    assert_eq!(overlay.version, 2);
    assert_eq!(overlay.text, "fn main() {\n    let new_var = 100;\n    let y = new_var;\n}\n");

    // Verify resolution of new_var at line 3, col 15
    let def = overlay.find_definition_at(3, 15).expect("resolve new_var");
    assert_eq!(def.name, "new_var");
    assert_eq!(def.start_line, 2);
}

#[test]
fn test_incremental_change_multiline_and_utf16() {
    // Line 0: fn test() {
    // Line 1:     let grüßen = "🚀";
    // Line 2: }
    let initial = "fn test() {\n    let grüßen = \"🚀\";\n}\n".to_string();
    let mut overlay = DocumentOverlay::new("file:///test/utf16.rs", "src/utf16.rs", 1, initial);

    // Replace "grüßen" (6 UTF-16 code units, starting at char 8) with "val"
    let change = serde_json::json!({
        "range": {
            "start": { "line": 1, "character": 8 },
            "end": { "line": 1, "character": 14 }
        },
        "text": "val"
    });

    overlay.apply_content_changes(2, &[change]);
    assert_eq!(overlay.text, "fn test() {\n    let val = \"🚀\";\n}\n");

    let def = overlay.find_definition_at(2, 10).expect("resolve val");
    assert_eq!(def.name, "val");
}

#[test]
fn test_incremental_change_full_fallback() {
    let initial = "fn a() {}\n".to_string();
    let mut overlay = DocumentOverlay::new("file:///test/full.rs", "src/full.rs", 1, initial);

    let change = serde_json::json!({
        "text": "fn b() {}\n"
    });

    overlay.apply_content_changes(2, &[change]);
    assert_eq!(overlay.text, "fn b() {}\n");
    assert_eq!(overlay.symbols.len(), 1);
    assert_eq!(overlay.symbols[0].name, "b");
}
