/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 3: Coordinate mapping, UTF-16 conversions, byte offsets, and diff projection tests.

use ridx::overlay::DocumentOverlay;

#[test]
fn test_map_line_identical_buffer() {
    let text = "line1\nline2\nline3\n";
    let ov = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, text.to_string());

    assert_eq!(ov.map_line_to_compiled(1), Some(1));
    assert_eq!(ov.map_line_to_compiled(2), Some(2));
    assert_eq!(ov.map_line_to_compiled(3), Some(3));
    assert_eq!(ov.map_line_to_compiled(4), None);
    assert_eq!(ov.map_compiled_line_to_live(1), Some(1));
    assert_eq!(ov.map_compiled_line_to_live(2), Some(2));
    assert_eq!(ov.map_compiled_line_to_live(3), Some(3));
    assert_eq!(ov.map_compiled_line_to_live(4), None);
}

#[test]
fn test_map_line_prefix_common_unaffected() {
    let compiled = "prefix1\nprefix2\nfn foo() {}\n";
    let live = "prefix1\nprefix2\nfn foo() {}\n// added at end\n";
    let mut ov = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, live.to_string());
    ov.advance_compiled_baseline(compiled.to_string());

    assert_eq!(ov.map_line_to_compiled(1), Some(1));
    assert_eq!(ov.map_line_to_compiled(2), Some(2));
    assert_eq!(ov.map_line_to_compiled(3), Some(3));
    assert_eq!(ov.map_line_to_compiled(4), None);
}

#[test]
fn test_map_line_suffix_common_shifted_after_insert() {
    let compiled = "line1\nline2\nline3\nline4\n";
    // Insert 2 lines between line1 and line2
    let live = "line1\ninserted_a\ninserted_b\nline2\nline3\nline4\n";
    let mut ov = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, live.to_string());
    ov.advance_compiled_baseline(compiled.to_string());

    assert_eq!(ov.map_line_to_compiled(1), Some(1));
    assert_eq!(ov.map_line_to_compiled(2), None);
    assert_eq!(ov.map_line_to_compiled(3), None);
    assert_eq!(ov.map_line_to_compiled(4), Some(2)); // line2 shifted from 2 to 4
    assert_eq!(ov.map_line_to_compiled(5), Some(3)); // line3 shifted from 3 to 5
    assert_eq!(ov.map_line_to_compiled(6), Some(4)); // line4 shifted from 4 to 6
}

#[test]
fn test_map_line_suffix_common_shifted_after_delete() {
    let compiled = "line1\ndelete_a\ndelete_b\nline2\nline3\n";
    // Delete lines 2 and 3
    let live = "line1\nline2\nline3\n";
    let mut ov = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, live.to_string());
    ov.advance_compiled_baseline(compiled.to_string());

    assert_eq!(ov.map_line_to_compiled(1), Some(1));
    assert_eq!(ov.map_line_to_compiled(2), Some(4)); // line2 was at line 4 in compiled
    assert_eq!(ov.map_line_to_compiled(3), Some(5)); // line3 was at line 5 in compiled
}

#[test]
fn test_map_compiled_line_to_live_after_insert() {
    let compiled = "line1\nline2\nline3\n";
    let live = "line1\ninserted_x\nline2\nline3\n";
    let mut ov = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, live.to_string());
    ov.advance_compiled_baseline(compiled.to_string());

    assert_eq!(ov.map_compiled_line_to_live(1), Some(1));
    assert_eq!(ov.map_compiled_line_to_live(2), Some(3)); // compiled line 2 shifted to 3
    assert_eq!(ov.map_compiled_line_to_live(3), Some(4)); // compiled line 3 shifted to 4
}

#[test]
fn test_map_compiled_line_to_live_after_delete() {
    let compiled = "line1\ndeleted_line\nline2\n";
    let live = "line1\nline2\n";
    let mut ov = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, live.to_string());
    ov.advance_compiled_baseline(compiled.to_string());

    assert_eq!(ov.map_compiled_line_to_live(1), Some(1));
    assert_eq!(ov.map_compiled_line_to_live(2), None); // deleted_line has no live counterpart
    assert_eq!(ov.map_compiled_line_to_live(3), Some(2));
}

#[test]
fn test_map_line_modified_middle_hunk_returns_none() {
    let compiled = "header\nold_logic_1\nold_logic_2\nfooter\n";
    let live = "header\nnew_logic_1\nnew_logic_2\nfooter\n";
    let mut ov = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, live.to_string());
    ov.advance_compiled_baseline(compiled.to_string());

    assert_eq!(ov.map_line_to_compiled(1), Some(1));
    assert_eq!(ov.map_line_to_compiled(2), None);
    assert_eq!(ov.map_line_to_compiled(3), None);
    assert_eq!(ov.map_line_to_compiled(4), Some(4));

    assert_eq!(ov.map_compiled_line_to_live(1), Some(1));
    assert_eq!(ov.map_compiled_line_to_live(2), None);
    assert_eq!(ov.map_compiled_line_to_live(3), None);
    assert_eq!(ov.map_compiled_line_to_live(4), Some(4));
}

#[test]
fn test_map_line_no_compiled_text_returns_none() {
    let ov = DocumentOverlay::new_with_provenance(
        "file:///test.rs",
        "/test.rs",
        1,
        "line1\nline2\n".to_string(),
        false,
    );

    assert_eq!(ov.map_line_to_compiled(1), None);
    assert_eq!(ov.map_line_to_compiled(2), None);
    assert_eq!(ov.map_compiled_line_to_live(1), None);
    assert_eq!(ov.map_live_to_compiled(1, 1), None);
    assert_eq!(ov.map_compiled_to_live(1, 1), None);
}

#[test]
fn test_map_line_overlapping_prefix_suffix_boundary() {
    // 3 identical lines in compiled vs 5 identical lines in live
    let compiled = "line\nline\nline\n";
    let live = "line\nline\nline\nline\nline\n";
    let mut ov = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, live.to_string());
    ov.advance_compiled_baseline(compiled.to_string());

    // Prefix matches 3 lines. Suffix cannot overlap prefix.
    let m1 = ov.map_line_to_compiled(1);
    let m2 = ov.map_line_to_compiled(2);
    let m3 = ov.map_line_to_compiled(3);
    let m4 = ov.map_line_to_compiled(4);
    let m5 = ov.map_line_to_compiled(5);

    assert_eq!(m1, Some(1));
    assert_eq!(m2, Some(2));
    assert_eq!(m3, Some(3));
    assert_eq!(m4, None);
    assert_eq!(m5, None);
}

#[test]
fn test_map_line_single_line_file() {
    let compiled = "single line";
    let live = "single line";
    let mut ov = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, live.to_string());
    ov.advance_compiled_baseline(compiled.to_string());

    assert_eq!(ov.map_line_to_compiled(1), Some(1));
    assert_eq!(ov.map_compiled_line_to_live(1), Some(1));

    ov.update(2, "edited single line".to_string());
    assert_eq!(ov.map_line_to_compiled(1), None);
    assert_eq!(ov.map_compiled_line_to_live(1), None);
}

#[test]
fn test_map_live_to_compiled_position_tuple() {
    let compiled = "line1\nline2\n";
    let live = "line1\nline2\n";
    let mut ov = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, live.to_string());
    ov.advance_compiled_baseline(compiled.to_string());

    assert_eq!(ov.map_live_to_compiled(2, 8), Some((2, 8)));
    assert_eq!(ov.map_live_to_compiled(99, 8), None);
}

#[test]
fn test_map_compiled_to_live_position_tuple() {
    let compiled = "line1\nline2\n";
    let live = "inserted\nline1\nline2\n";
    let mut ov = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, live.to_string());
    ov.advance_compiled_baseline(compiled.to_string());

    assert_eq!(ov.map_compiled_to_live(1, 5), Some((2, 5)));
    assert_eq!(ov.map_compiled_to_live(2, 5), Some((3, 5)));
}

#[test]
fn test_has_modified_imports_add_remove_change() {
    let base = "use std::sync::Arc;\nfn run() {}\n";
    let mut ov = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, base.to_string());
    ov.advance_compiled_baseline(base.to_string());
    assert!(!ov.has_modified_imports());

    // Add import
    ov.update(
        2,
        "use std::sync::Arc;\nuse std::rc::Rc;\nfn run() {}\n".to_string(),
    );
    assert!(ov.has_modified_imports());

    // Remove import
    ov.update(3, "fn run() {}\n".to_string());
    assert!(ov.has_modified_imports());

    // Only whitespace / comments
    ov.update(
        4,
        "// comment\nuse std::sync::Arc;\nfn run() {}\n".to_string(),
    );
    assert!(!ov.has_modified_imports());
}

#[test]
fn test_position_to_byte_offset_ascii_lines() {
    let text = "alpha\nbeta\ngamma\n";
    let ov = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, text.to_string());

    assert_eq!(ov.position_to_byte_offset(0, 0), 0);
    assert_eq!(ov.position_to_byte_offset(0, 3), 3);
    assert_eq!(ov.position_to_byte_offset(1, 0), 6);
    assert_eq!(ov.position_to_byte_offset(1, 4), 10);
    assert_eq!(ov.position_to_byte_offset(2, 0), 11);
}

#[test]
fn test_position_to_byte_offset_surrogate_pair_emoji() {
    // 🚀 is 4 bytes UTF-8, 2 code units in UTF-16
    let text = "let x = '🚀';\n";
    let ov = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, text.to_string());

    // 'let x = \'' is 9 bytes ASCII
    assert_eq!(ov.position_to_byte_offset(0, 9), 9);
    // After 🚀 in UTF-16 (character 11 = 9 + 2): byte offset is 9 + 4 = 13
    assert_eq!(ov.position_to_byte_offset(0, 11), 13);
}

#[test]
fn test_position_to_byte_offset_multibyte_greek() {
    // "αβγδ" is 4 characters, 8 bytes in UTF-8, 4 code units in UTF-16
    let text = "αβγδ\n";
    let ov = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, text.to_string());

    assert_eq!(ov.position_to_byte_offset(0, 0), 0);
    assert_eq!(ov.position_to_byte_offset(0, 1), 2);
    assert_eq!(ov.position_to_byte_offset(0, 2), 4);
    assert_eq!(ov.position_to_byte_offset(0, 4), 8);
}

#[test]
fn test_byte_offset_to_position_roundtrip() {
    let text = "fn foo() {\n    let val = \"Hello, 🚀\";\n}\n";
    let ov = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, text.to_string());

    for (b_idx, c) in text.char_indices() {
        let (l, ch) = ov.byte_offset_to_position(b_idx);
        let recovered = ov.position_to_byte_offset(l, ch);
        assert_eq!(
            recovered, b_idx,
            "Failed roundtrip at byte {b_idx} (char '{c}'): got line {l}, char {ch}"
        );
    }
}

#[test]
fn test_utf16_col_to_char_col_and_char_to_utf16() {
    let text = "let msg = \"🦀 Rust\";\n";
    let ov = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, text.to_string());

    // 0-based UTF-16 char 11 (at start of '🦀', 11 chars after 'let msg = "')
    let char_col = ov.utf16_col_to_char_col(1, 11);
    assert_eq!(char_col, 12); // 1-based col 12

    let utf16_col = ov.char_col_to_utf16_col(1, char_col);
    assert_eq!(utf16_col, 11);
}
