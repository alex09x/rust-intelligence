/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use crate::overlay::mapping::*;
use crate::overlay::DocumentOverlay;

#[test]
fn test_strip_whitespace_and_comments() {
    assert_eq!(strip_trailing_whitespace_and_comments("code; // comment\n  "), "code;");
    assert_eq!(strip_trailing_whitespace_and_comments("code; /* block */ "), "code;");
    assert_eq!(strip_leading_whitespace_and_comments("  // comment\ncode;"), "code;");
    assert_eq!(strip_leading_whitespace_and_comments(" /* block */ code;"), "code;");
}

#[test]
fn test_is_byte_in_string_or_comment() {
    let line = "let x = \"hello\"; // comment";
    assert!(is_byte_in_string_or_comment(line, 9)); // inside "hello"
    assert!(is_byte_in_string_or_comment(line, 20)); // inside // comment
    assert!(!is_byte_in_string_or_comment(line, 4)); // at 'x'
}

#[test]
fn test_mapping_clean_and_dirty() {
    let base = "line1\nline2\nline3\n";
    let mut ov = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, base.to_string());
    assert!(!ov.is_dirty());
    assert_eq!(ov.map_line_to_compiled(2), Some(2));
    assert_eq!(ov.map_compiled_line_to_live(2), Some(2));

    // Insert line at beginning
    let dirty = "inserted\nline1\nline2\nline3\n";
    ov.update(2, dirty.to_string());
    assert!(ov.is_dirty());
    assert_eq!(ov.map_line_to_compiled(3), Some(2));
    assert_eq!(ov.map_compiled_line_to_live(2), Some(3));
}

#[test]
fn test_has_modified_imports() {
    let base = "// common prefix\nuse std::collections::HashMap;\nfn test() {}\n";
    let mut ov = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, base.to_string());
    assert!(!ov.has_modified_imports());

    ov.update(2, "// common prefix\nuse std::collections::HashSet;\nfn test() {}\n".to_string());
    assert!(ov.has_modified_imports());

    // pub use, mod, extern crate
    ov.update(3, "// common prefix\npub use std::path::Path;\nfn test() {}\n".to_string());
    assert!(ov.has_modified_imports());
    ov.update(4, "// common prefix\nmod my_mod;\nfn test() {}\n".to_string());
    assert!(ov.has_modified_imports());
    ov.update(5, "// common prefix\nextern crate my_crate;\nfn test() {}\n".to_string());
    assert!(ov.has_modified_imports());
}

#[test]
fn test_is_byte_in_string_or_comment_extended() {
    let s = "let a = /* comment */ 42; let b = \"escaped \\\" quote\"; let c = 'x';";
    assert!(is_byte_in_string_or_comment(s, 12)); // inside /* comment */
    assert!(is_byte_in_string_or_comment(s, 40)); // inside "escaped \" quote"
    assert!(is_byte_in_string_or_comment(s, 64)); // inside 'x'
    assert!(!is_byte_in_string_or_comment(s, 23)); // at '4'

    let unclosed_block = "/* unclosed block comment";
    assert!(is_byte_in_string_or_comment(unclosed_block, 10));
}

#[test]
fn test_mapping_edge_cases() {
    // 1. compiled_text is None
    let mut ov_no_compiled = DocumentOverlay::new_with_provenance("file:///test.rs", "/test.rs", 1, "line1\nline2\n".to_string(), false);
    assert_eq!(ov_no_compiled.map_line_to_compiled(1), None);
    assert_eq!(ov_no_compiled.map_line_to_compiled(999), None);
    assert_eq!(ov_no_compiled.map_compiled_line_to_live(1), None);
    assert_eq!(ov_no_compiled.map_compiled_line_to_live(999), None);
    assert_eq!(ov_no_compiled.map_live_to_compiled(1, 5), None);
    assert_eq!(ov_no_compiled.map_compiled_to_live(1, 5), None);

    // 2. compiled_text equal to text
    ov_no_compiled.advance_compiled_baseline("line1\nline2\n".to_string());
    assert_eq!(ov_no_compiled.map_line_to_compiled(1), Some(1));
    assert_eq!(ov_no_compiled.map_line_to_compiled(999), None);
    assert_eq!(ov_no_compiled.map_compiled_line_to_live(1), Some(1));
    assert_eq!(ov_no_compiled.map_compiled_line_to_live(999), None);

    // 3. Middle modified lines return None
    let mut ov_diff = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, "top\nmid_live\nbot\n".to_string());
    ov_diff.advance_compiled_baseline("top\nmid_comp1\nmid_comp2\nbot\n".to_string());
    assert_eq!(ov_diff.map_compiled_line_to_live(1), Some(1));
    assert_eq!(ov_diff.map_compiled_line_to_live(2), None);
    assert_eq!(ov_diff.map_compiled_line_to_live(3), None);
    assert_eq!(ov_diff.map_line_to_compiled(1), Some(1));
    assert_eq!(ov_diff.map_line_to_compiled(2), None);

    // 4. Nested block comment stripping
    assert_eq!(strip_leading_whitespace_and_comments("/* /* nested */ */ code"), "code");

    // 5. Multiple trailing block comments and unmatched */ (line 46)
    assert_eq!(strip_trailing_whitespace_and_comments("code; /* c1 */ /* c2 */ "), "code;");
    assert_eq!(strip_trailing_whitespace_and_comments("code; */ "), "code; */");

    // 6. has_modified_imports with None compiled_text and pure deletion (lines 280, 309)
    let ov_no_comp = DocumentOverlay::new_with_provenance("file:///test.rs", "/test.rs", 1, "text\n".to_string(), false);
    assert!(!ov_no_comp.has_modified_imports());

    let mut ov_del = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, "use std::io;\nfn f() {}\n".to_string());
    ov_del.advance_compiled_baseline("use std::io;\nfn f() {}\n".to_string());
    ov_del.update(2, "fn f() {}\n".to_string());
    assert!(ov_del.has_modified_imports());
}

#[test]
fn test_overlay_query_additional_coverage() {
    // 1. get_ident_at on an empty line (line 145 in query.rs)
    let ov_empty = DocumentOverlay::new("file:///empty.rs", "src/empty.rs", 1, "\n\n".to_string());
    assert_eq!(ov_empty.get_ident_at(1, 1), None);

    // 2. find_local_binding_references with substring ident matches and shadowing (lines 241, 257)
    let code = "fn test() {\n    let val = 1;\n    let extra_val_var = 2;\n    { let val = 3; val; }\n    val;\n}\n";
    let ov_binding = DocumentOverlay::new("file:///binding.rs", "src/binding.rs", 1, code.to_string());
    let refs = ov_binding.find_local_binding_references(2, 9);
    assert!(!refs.is_empty());
}
