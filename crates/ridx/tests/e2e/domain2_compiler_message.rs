/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 2: Unit tests for cargo check JSON message parsing into LSP diagnostics,
//! multi-span locations, UTF-16 code units, and compiler child notes.

use ridx::lsp::presentation::compiler_message::{char_col_to_utf16, parse_compiler_message};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

fn setup_temp_workspace() -> (PathBuf, PathBuf) {
    let id = TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_dir =
        std::env::temp_dir().join(format!("ridx_test_d2_cm_{}_{}", std::process::id(), id));
    let src_file = temp_dir.join("src/lib.rs");
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let _ = fs::write(&src_file, "pub fn example() {}\n");
    (temp_dir, src_file)
}

#[test]
fn test_parse_simple_compiler_error() {
    let (root, _) = setup_temp_workspace();
    let raw = json!({
        "message": "cannot find value `foo` in this scope",
        "code": { "code": "E0425" },
        "level": "error",
        "spans": [{
            "file_name": "src/lib.rs",
            "line_start": 2,
            "line_end": 2,
            "column_start": 5,
            "column_end": 8,
            "is_primary": true,
            "text": [{ "text": "    foo", "highlight_start": 5, "highlight_end": 8 }]
        }],
        "children": []
    });

    let res = parse_compiler_message(&raw, &root).expect("Should parse");
    let (uri, diag) = res;
    assert!(uri.starts_with("file://"));
    assert!(uri.ends_with("src/lib.rs"));
    assert_eq!(diag["severity"], 1); // 1 = Error
    assert_eq!(diag["code"], "E0425");
    assert_eq!(diag["source"], "rustc");
    assert_eq!(diag["range"]["start"]["line"], 1); // 0-based
    assert_eq!(diag["range"]["start"]["character"], 4); // 0-based
    assert_eq!(diag["range"]["end"]["line"], 1);
    assert_eq!(diag["range"]["end"]["character"], 7);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_parse_compiler_warning_and_severities() {
    let (root, _) = setup_temp_workspace();
    let severities = [
        ("warning", 2),
        ("note", 3),
        ("help", 4),
        ("unknown_level", 1),
    ];

    for (level, expected_sev) in severities {
        let raw = json!({
            "message": format!("test {level}"),
            "code": { "code": "unused_variables" },
            "level": level,
            "spans": [{
                "file_name": "src/lib.rs",
                "line_start": 1,
                "line_end": 1,
                "column_start": 1,
                "column_end": 4,
                "is_primary": true,
                "text": [{ "text": "pub", "highlight_start": 1, "highlight_end": 4 }]
            }]
        });
        let (_, diag) = parse_compiler_message(&raw, &root).unwrap();
        assert_eq!(diag["severity"], expected_sev);
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_parse_multiline_span_start_and_end() {
    let (root, _) = setup_temp_workspace();
    let raw = json!({
        "message": "mismatched closing delimiter",
        "level": "error",
        "spans": [{
            "file_name": "src/lib.rs",
            "line_start": 3,
            "line_end": 7,
            "column_start": 5,
            "column_end": 10,
            "is_primary": true,
            "text": [
                { "text": "    fn multiline() {", "highlight_start": 5, "highlight_end": 21 },
                { "text": "        let x = 1;", "highlight_start": 1, "highlight_end": 19 },
                { "text": "    } // done", "highlight_start": 1, "highlight_end": 10 }
            ]
        }]
    });

    let (_, diag) = parse_compiler_message(&raw, &root).unwrap();
    assert_eq!(diag["range"]["start"]["line"], 2);
    assert_eq!(diag["range"]["start"]["character"], 4);
    assert_eq!(diag["range"]["end"]["line"], 6);
    assert_eq!(diag["range"]["end"]["character"], 9);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_parse_utf16_columns_with_astral_emojis() {
    let (root, _) = setup_temp_workspace();
    // "/* 🦀 */ let x = 1;"
    // "/* " -> 3 chars / 3 UTF-16
    // "🦀"  -> 1 char / 2 UTF-16 (surrogate pair)
    // " */ let " -> 8 chars / 8 UTF-16
    // Char col before x: 3 + 1 + 8 = 12 chars (1-based: 13)
    // UTF-16 col before x: 3 + 2 + 8 = 13 code units (0-based: 13)
    let line_text = "/* 🦀 */ let x = 1;";
    let raw = json!({
        "message": "unused variable: `x`",
        "level": "warning",
        "spans": [{
            "file_name": "src/lib.rs",
            "line_start": 1,
            "line_end": 1,
            "column_start": 13, // 1-based char column of `x`
            "column_end": 14,
            "is_primary": true,
            "text": [{
                "text": line_text,
                "highlight_start": 13,
                "highlight_end": 14
            }]
        }]
    });

    let (_, diag) = parse_compiler_message(&raw, &root).unwrap();
    assert_eq!(diag["range"]["start"]["character"], 13); // UTF-16 converted (+1 unit for 🦀 surrogate)
    assert_eq!(diag["range"]["end"]["character"], 14);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_parse_utf16_multibyte_greek_and_german() {
    let (root, _) = setup_temp_workspace();
    // BMP non-ASCII characters: "grüßen" and "δ" are each 1 UTF-16 code unit
    let line_text = "let δ = \"grüßen\";";
    let raw = json!({
        "message": "greek identifier",
        "level": "note",
        "spans": [{
            "file_name": "src/lib.rs",
            "line_start": 1,
            "line_end": 1,
            "column_start": 5,
            "column_end": 6,
            "is_primary": true,
            "text": [{ "text": line_text, "highlight_start": 5, "highlight_end": 6 }]
        }]
    });

    let (_, diag) = parse_compiler_message(&raw, &root).unwrap();
    assert_eq!(diag["range"]["start"]["character"], 4);
    assert_eq!(diag["range"]["end"]["character"], 5);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_parse_related_information_secondary_spans() {
    let (root, _) = setup_temp_workspace();
    let raw = json!({
        "message": "cannot borrow `v` as mutable more than once at a time",
        "code": { "code": "E0499" },
        "level": "error",
        "spans": [
            {
                "file_name": "src/lib.rs",
                "line_start": 10,
                "line_end": 10,
                "column_start": 5,
                "column_end": 12,
                "is_primary": true,
                "text": [{ "text": "    v.push(1);", "highlight_start": 5, "highlight_end": 12 }]
            },
            {
                "file_name": "src/lib.rs",
                "line_start": 8,
                "line_end": 8,
                "column_start": 14,
                "column_end": 15,
                "is_primary": false,
                "label": "first mutable borrow occurs here",
                "text": [{ "text": "    let r1 = &mut v;", "highlight_start": 14, "highlight_end": 15 }]
            }
        ]
    });

    let (_, diag) = parse_compiler_message(&raw, &root).unwrap();
    let rel = diag["relatedInformation"]
        .as_array()
        .expect("relatedInformation array");
    assert_eq!(rel.len(), 1);
    assert_eq!(rel[0]["message"], "first mutable borrow occurs here");
    assert_eq!(rel[0]["location"]["range"]["start"]["line"], 7); // line 8 -> 7
    assert_eq!(rel[0]["location"]["range"]["start"]["character"], 13); // col 14 -> 13
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_parse_children_notes_and_helps() {
    let (root, _) = setup_temp_workspace();
    let raw = json!({
        "message": "mismatched types",
        "code": { "code": "E0308" },
        "level": "error",
        "spans": [{
            "file_name": "src/lib.rs",
            "line_start": 4,
            "line_end": 4,
            "column_start": 10,
            "column_end": 15,
            "is_primary": true,
            "text": [{ "text": "    let x = \"foo\";", "highlight_start": 10, "highlight_end": 15 }]
        }],
        "children": [
            {
                "level": "note",
                "message": "expected `i32`, found `&str`"
            },
            {
                "level": "help",
                "message": "try parsing the string: `\"foo\".parse::<i32>()`"
            }
        ]
    });

    let (_, diag) = parse_compiler_message(&raw, &root).unwrap();
    let msg = diag["message"].as_str().unwrap();
    assert!(msg.contains("mismatched types"));
    assert!(msg.contains("note: expected `i32`, found `&str`"));
    assert!(msg.contains("help: try parsing the string"));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_parse_macro_expansion_span_preservation() {
    let (root, _) = setup_temp_workspace();
    let raw = json!({
        "message": "format argument must be a string literal",
        "level": "error",
        "spans": [{
            "file_name": "src/lib.rs",
            "line_start": 5,
            "line_end": 5,
            "column_start": 5,
            "column_end": 18,
            "is_primary": true,
            "text": [{ "text": "    println!(x);", "highlight_start": 5, "highlight_end": 18 }],
            "expansion": {
                "span": {
                    "file_name": "<println macros>",
                    "line_start": 1,
                    "column_start": 1,
                    "line_end": 1,
                    "column_end": 10
                },
                "macro_decl_name": "println!"
            }
        }]
    });

    let res = parse_compiler_message(&raw, &root);
    assert!(res.is_some(), "Should parse macro invocation span");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_parse_missing_code_field_produces_null_code() {
    let (root, _) = setup_temp_workspace();
    let raw = json!({
        "message": "aborting due to previous error",
        "level": "error",
        "spans": [{
            "file_name": "src/lib.rs",
            "line_start": 1,
            "line_end": 1,
            "column_start": 1,
            "column_end": 5,
            "is_primary": true,
            "text": [{ "text": "code", "highlight_start": 1, "highlight_end": 5 }]
        }]
    });

    let (_, diag) = parse_compiler_message(&raw, &root).unwrap();
    assert!(diag["code"].is_null());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_parse_no_primary_span_or_empty_spans_returns_none() {
    let (root, _) = setup_temp_workspace();
    let no_spans = json!({
        "message": "linking with `cc` failed: exit status: 1",
        "level": "error",
        "spans": []
    });
    assert!(parse_compiler_message(&no_spans, &root).is_none());

    let secondary_only = json!({
        "message": "some info",
        "level": "note",
        "spans": [{
            "file_name": "src/lib.rs",
            "line_start": 1,
            "line_end": 1,
            "column_start": 1,
            "column_end": 2,
            "is_primary": false
        }]
    });
    assert!(parse_compiler_message(&secondary_only, &root).is_none());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_char_col_to_utf16_unit_boundaries() {
    // ASCII only: 1 char = 1 code unit
    assert_eq!(char_col_to_utf16("hello world", 0), 0);
    assert_eq!(char_col_to_utf16("hello world", 5), 5);
    assert_eq!(char_col_to_utf16("hello world", 20), 11); // clamped to length

    // Surrogate pairs: 🦀 is 2 UTF-16 code units
    let s = "🦀_crab";
    assert_eq!(char_col_to_utf16(s, 0), 0);
    assert_eq!(char_col_to_utf16(s, 1), 2); // after 🦀
    assert_eq!(char_col_to_utf16(s, 2), 3); // after _
    assert_eq!(char_col_to_utf16(s, 3), 4); // after c

    // Two emojis in sequence: 🦀🚀
    let two_emojis = "🦀🚀_rocket";
    assert_eq!(char_col_to_utf16(two_emojis, 0), 0);
    assert_eq!(char_col_to_utf16(two_emojis, 1), 2);
    assert_eq!(char_col_to_utf16(two_emojis, 2), 4);
    assert_eq!(char_col_to_utf16(two_emojis, 3), 5);
}
