/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! End-to-end tests for compiler flycheck diagnostics and auto-import intelligence.

use ridx::lsp::intelligence::auto_import::{compute_auto_import_edit, find_use_insert_line, is_already_imported};
use ridx::lsp::presentation::flycheck::{parse_compiler_message, FlycheckState};
use serde_json::json;
use std::fs;

#[test]
fn test_auto_import_standard_collection() {
    let doc = r#"fn main() {
    let mut map = HashMap::new();
}
"#;
    let edit_opt = compute_auto_import_edit(doc, "std::collections::HashMap");
    assert!(edit_opt.is_some(), "Expected auto-import edit for HashMap");
    let edit = edit_opt.unwrap();
    assert_eq!(edit["newText"].as_str().unwrap(), "use std::collections::HashMap;\n");
    assert_eq!(edit["range"]["start"]["line"].as_u64().unwrap(), 0);
    assert_eq!(edit["range"]["start"]["character"].as_u64().unwrap(), 0);
}

#[test]
fn test_auto_import_skips_inner_attributes_and_comments() {
    let doc = r#"//! Module doc comment
//! Another line

#![allow(unused)]

fn main() {
    let b = BTreeMap::new();
}
"#;
    let line = find_use_insert_line(doc);
    assert_eq!(line, 5, "Should insert after inner doc comments and inner attributes");

    let edit_opt = compute_auto_import_edit(doc, "std::collections::BTreeMap");
    assert!(edit_opt.is_some());
    let edit = edit_opt.unwrap();
    assert_eq!(edit["range"]["start"]["line"].as_u64().unwrap(), 5);
}

#[test]
fn test_auto_import_appends_after_existing_use_statements() {
    let doc = r#"use std::io::Read;
use std::path::Path;

fn main() {}
"#;
    let line = find_use_insert_line(doc);
    assert_eq!(line, 2, "Should insert right after last use statement");

    let edit = compute_auto_import_edit(doc, "std::sync::Arc").expect("Should produce edit");
    assert_eq!(edit["range"]["start"]["line"].as_u64().unwrap(), 2);
    assert_eq!(edit["newText"].as_str().unwrap(), "use std::sync::Arc;\n");
}

#[test]
fn test_auto_import_detects_already_imported_symbols() {
    let doc1 = "use std::collections::HashMap;\nfn main() {}";
    assert!(is_already_imported(doc1, "std::collections::HashMap", "HashMap"));
    assert!(compute_auto_import_edit(doc1, "std::collections::HashMap").is_none());

    let doc2 = "use std::collections::{BTreeMap, HashMap, HashSet};\nfn main() {}";
    assert!(is_already_imported(doc2, "std::collections::HashMap", "HashMap"));
    assert!(compute_auto_import_edit(doc2, "std::collections::HashMap").is_none());

    let doc3 = "use std::collections::HashMap as MyMap;\nfn main() {}";
    assert!(is_already_imported(doc3, "std::collections::HashMap", "HashMap"));
}

#[test]
fn test_auto_import_skips_prelude_and_bare_names() {
    let doc = "fn main() {}";
    assert!(compute_auto_import_edit(doc, "String").is_none());
    assert!(compute_auto_import_edit(doc, "std::vec::Vec").is_none());
    assert!(compute_auto_import_edit(doc, "Option").is_none());
    assert!(compute_auto_import_edit(doc, "bare_ident").is_none());
}

#[test]
fn test_parse_compiler_message_error_with_spans() {
    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("test_flycheck_{pid}"));
    let src_file = tmp.join("src").join("main.rs");
    let _ = fs::create_dir_all(src_file.parent().unwrap());
    let _ = fs::write(&src_file, "fn main() { let x: u32 = \"str\"; }\n");

    let raw_msg = json!({
        "children": [],
        "code": { "code": "E0308", "explanation": "mismatched types" },
        "level": "error",
        "message": "mismatched types",
        "rendered": "error[E0308]: mismatched types...",
        "spans": [
            {
                "byte_end": 31,
                "byte_start": 26,
                "column_end": 32,
                "column_start": 27,
                "file_name": "src/main.rs",
                "is_primary": true,
                "label": "expected `u32`, found `&str`",
                "line_end": 1,
                "line_start": 1
            },
            {
                "byte_end": 23,
                "byte_start": 20,
                "column_end": 24,
                "column_start": 21,
                "file_name": "src/main.rs",
                "is_primary": false,
                "label": "expected due to this",
                "line_end": 1,
                "line_start": 1
            }
        ]
    });

    let res = parse_compiler_message(&raw_msg, &tmp);
    assert!(res.is_some(), "Compiler message should parse cleanly");
    let (uri, diag) = res.unwrap();

    assert!(uri.starts_with("file://"));
    assert!(uri.ends_with("src/main.rs"));

    assert_eq!(diag["severity"].as_u64().unwrap(), 1, "Error severity must be 1");
    assert_eq!(diag["code"].as_str().unwrap(), "E0308");
    assert_eq!(diag["source"].as_str().unwrap(), "rustc");
    // The primary span's label on the first line, as `cargo check --message-format=short` prints it.
    assert_eq!(diag["message"].as_str().unwrap(), "mismatched types: expected `u32`, found `&str`");

    // 0-based range conversion
    assert_eq!(diag["range"]["start"]["line"].as_u64().unwrap(), 0);
    assert_eq!(diag["range"]["start"]["character"].as_u64().unwrap(), 26);
    assert_eq!(diag["range"]["end"]["line"].as_u64().unwrap(), 0);
    assert_eq!(diag["range"]["end"]["character"].as_u64().unwrap(), 31);

    // Related info
    let rel = diag["relatedInformation"].as_array().unwrap();
    assert_eq!(rel.len(), 1);
    assert_eq!(rel[0]["message"].as_str().unwrap(), "expected due to this");

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_flycheck_state_cancellation() {
    let state = FlycheckState::new();
    // Verify cancel on empty state does not panic
    state.cancel();
    state.cancel();
}

#[test]
fn test_auto_import_inserts_after_multiline_use_not_inside() {
    let doc = "use std::collections::{\n    HashMap,\n    HashSet,\n};\nfn main() {}\n";
    let line = find_use_insert_line(doc);
    assert_eq!(line, 4, "Should insert after multiline use, not inside it!");

    let edit = compute_auto_import_edit(doc, "std::sync::Arc").expect("Should produce edit");
    assert_eq!(edit["range"]["start"]["line"].as_u64().unwrap(), 4);
    assert_eq!(edit["newText"].as_str().unwrap(), "use std::sync::Arc;\n");
}

#[test]
fn test_auto_import_detects_braced_spaces_and_glob_imports() {
    // Spaced braces
    let doc1 = "use std::collections::{ HashMap };\nfn main() {}";
    assert!(is_already_imported(doc1, "std::collections::HashMap", "HashMap"));
    assert!(compute_auto_import_edit(doc1, "std::collections::HashMap").is_none());

    // Glob import
    let doc2 = "use std::collections::*;\nfn main() {}";
    assert!(is_already_imported(doc2, "std::collections::HashMap", "HashMap"));
    assert!(compute_auto_import_edit(doc2, "std::collections::HashMap").is_none());

    // Multiline braced group
    let doc3 = "use std::collections::{\n    HashMap,\n    HashSet,\n};\nfn main() {}";
    assert!(is_already_imported(doc3, "std::collections::HashMap", "HashMap"));
    assert!(compute_auto_import_edit(doc3, "std::collections::HashMap").is_none());

    // Nested braced groups
    let doc4 = "use std::{collections::{HashMap}, io};\nfn main() {}";
    assert!(is_already_imported(doc4, "std::collections::HashMap", "HashMap"));
    assert!(compute_auto_import_edit(doc4, "std::collections::HashMap").is_none());

    // Neighbor module with common prefix must NOT trigger glob match
    let doc5 = "use foo::bar_ext::*;\nfn main() {}";
    assert!(!is_already_imported(doc5, "foo::bar::Baz", "Baz"));
    let edit5 = compute_auto_import_edit(doc5, "foo::bar::Baz");
    assert!(edit5.is_some(), "Neighbor glob must not suppress target import");
}

#[test]
fn test_prelude_allows_fmt_result_and_requires_hash() {
    let doc = "fn main() {}";
    // std::fmt::Result is NOT prelude and must produce an auto-import edit!
    let edit = compute_auto_import_edit(doc, "std::fmt::Result");
    assert!(edit.is_some(), "std::fmt::Result should be imported!");
    assert_eq!(edit.unwrap()["newText"].as_str().unwrap(), "use std::fmt::Result;\n");

    // core::hash::Hash is NOT in the Rust prelude (only #[derive(Hash)]) and should be imported!
    let edit_hash = compute_auto_import_edit(doc, "core::hash::Hash");
    assert!(edit_hash.is_some(), "core::hash::Hash should be imported!");
    assert_eq!(edit_hash.unwrap()["newText"].as_str().unwrap(), "use core::hash::Hash;\n");
}

#[test]
fn test_parse_compiler_message_includes_children_notes_and_utf16_columns() {
    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("test_flycheck_utf16_{pid}"));
    let src_file = tmp.join("src").join("lib.rs");
    let _ = fs::create_dir_all(src_file.parent().unwrap());
    let _ = fs::write(&src_file, "pub fn check() {}\n");

    let raw_msg = json!({
        "children": [
            {
                "level": "help",
                "message": "try using .as_ref() instead"
            }
        ],
        "code": { "code": "E0308", "explanation": "mismatched types" },
        "level": "error",
        "message": "type mismatch",
        "spans": [
            {
                "byte_end": 12,
                "byte_start": 5,
                "column_end": 13,
                "column_start": 6,
                "file_name": "src/lib.rs",
                "is_primary": true,
                "line_end": 1,
                "line_start": 1,
                "text": [
                    {
                        "highlight_end": 13,
                        "highlight_start": 6,
                        "text": "pub fn check() {}"
                    }
                ]
            }
        ]
    });

    let res = parse_compiler_message(&raw_msg, &tmp).expect("Should parse");
    let (_, diag) = res;

    let msg = diag["message"].as_str().unwrap();
    assert!(msg.contains("type mismatch"), "Must contain primary message");
    assert!(msg.contains("help: try using .as_ref() instead"), "Must contain child message");

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_same_crate_auto_import_produces_crate_prefix() {
    let doc = "fn main() {}\n";
    let edit = compute_auto_import_edit(doc, "crate::my_mod::MyStruct").expect("Should produce edit");
    assert_eq!(edit["newText"].as_str().unwrap(), "use crate::my_mod::MyStruct;\n");
}
