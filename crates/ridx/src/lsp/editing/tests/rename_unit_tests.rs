/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unit tests for local overlay renaming with syn::parse_file oracle.
//!
//! Verifies that local variable renames accurately transform overlay buffers,
//! preserve CRLF, respect lexical scoping and shadowing, leave comments and
//! string literals untouched, and always yield syntactically valid Rust.

use crate::lsp::editing::rename::handle_rename;
use crate::lsp::protocol::{path_to_uri, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_test_support::{apply_workspace_edit, parse_fixture};
use serde_json::{json, Value};
use std::collections::HashMap;

fn run_local_rename_result(
    fixture: &str,
    new_name: &str,
) -> Result<(Value, String, HashMap<String, String>), crate::lsp::error::LspError> {
    let parsed = parse_fixture(fixture);
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("lib.rs");
    let file_str = file.to_str().unwrap();
    let uri = path_to_uri(file_str);

    let mut overlay_mgr = OverlayManager::new();
    overlay_mgr.open_document(&uri, file_str, 1, parsed.clean_text.clone());
    let mut conv = PositionConverter::new();

    let cur = parsed.cursor.expect("Fixture must specify a cursor $0");
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": cur.line, "character": cur.col_utf16 },
            "newName": new_name
        }
    });

    let res = handle_rename(&req, None, &overlay_mgr, &mut conv)?;
    let mut files = HashMap::new();
    files.insert(uri.clone(), parsed.clean_text);

    let updated = apply_workspace_edit(&files, &res);
    Ok((res, uri, updated))
}

fn run_local_rename(fixture: &str, new_name: &str) -> (Value, String, HashMap<String, String>) {
    run_local_rename_result(fixture, new_name).expect("rename succeeds")
}

#[test]
fn test_rename_local_variable_oracle_syntax() {
    let fixture = "fn compute() -> i32 {\n    let $0x = 10;\n    let y = x + 5;\n    x * y\n}\n";
    let (_res, uri, updated) = run_local_rename(fixture, "multiplier");
    let new_code = &updated[&uri];

    assert_eq!(
        new_code,
        "fn compute() -> i32 {\n    let multiplier = 10;\n    let y = multiplier + 5;\n    multiplier * y\n}\n"
    );
    syn::parse_file(new_code).expect("Resulting code must be valid Rust");
}

#[test]
fn test_rename_function_argument() {
    let fixture = "fn scale(fac$0tor: f64, val: f64) -> f64 {\n    val * factor\n}\n";
    let (_res, uri, updated) = run_local_rename(fixture, "multiplier");
    let new_code = &updated[&uri];

    assert_eq!(
        new_code,
        "fn scale(multiplier: f64, val: f64) -> f64 {\n    val * multiplier\n}\n"
    );
    syn::parse_file(new_code).expect("Valid Rust");
}

#[test]
fn test_rename_mutable_let_binding() {
    let fixture = "fn process() -> i32 {\n    let mut $0counter = 0;\n    counter += 1;\n    counter * 2\n}\n";
    let (_res, uri, updated) = run_local_rename(fixture, "total");
    let new_code = &updated[&uri];

    assert_eq!(
        new_code,
        "fn process() -> i32 {\n    let mut total = 0;\n    total += 1;\n    total * 2\n}\n"
    );
    syn::parse_file(new_code).expect("Valid Rust");
}

#[test]
fn test_rename_multiple_occurrences_single_expr() {
    let fixture = "fn poly(x: i32) -> i32 {\n    let $0a = 2;\n    a * a + a\n}\n";
    let (_res, uri, updated) = run_local_rename(fixture, "coeff");
    let new_code = &updated[&uri];

    assert_eq!(
        new_code,
        "fn poly(x: i32) -> i32 {\n    let coeff = 2;\n    coeff * coeff + coeff\n}\n"
    );
    syn::parse_file(new_code).expect("Valid Rust");
}

#[test]
fn test_rename_shadowed_inner_variable() {
    let fixture = "fn shadow() {\n    let x = 1;\n    {\n        let $0x = 2;\n        let _ = x + 1;\n    }\n    let _ = x;\n}\n";
    let (_res, uri, updated) = run_local_rename(fixture, "inner_x");
    let new_code = &updated[&uri];

    assert_eq!(
        new_code,
        "fn shadow() {\n    let x = 1;\n    {\n        let inner_x = 2;\n        let _ = inner_x + 1;\n    }\n    let _ = x;\n}\n"
    );
    syn::parse_file(new_code).expect("Valid Rust");
}

#[test]
fn test_rename_crlf_document_preserves_endings() {
    let fixture = "fn test() {\r\n    let $0item = 100;\r\n    let res = item * 2;\r\n}\r\n";
    let (_res, uri, updated) = run_local_rename(fixture, "count");
    let new_code = &updated[&uri];

    assert_eq!(
        new_code,
        "fn test() {\r\n    let count = 100;\r\n    let res = count * 2;\r\n}\r\n"
    );
    syn::parse_file(new_code).expect("Valid Rust");
}

#[test]
fn test_rename_skips_comments_and_strings() {
    let fixture = "fn test() {\n    // x should not change\n    let $0x = 42;\n    let msg = \"value of x is here\";\n    let y = x;\n}\n";
    let (_res, uri, updated) = run_local_rename(fixture, "target_var");
    let new_code = &updated[&uri];

    assert_eq!(
        new_code,
        "fn test() {\n    // x should not change\n    let target_var = 42;\n    let msg = \"value of x is here\";\n    let y = target_var;\n}\n"
    );
    syn::parse_file(new_code).expect("Valid Rust");
}

#[test]
fn test_rename_local_with_type_annotation() {
    let fixture = "fn calc() -> usize {\n    let $0size: usize = 1024;\n    size * 2\n}\n";
    let (_res, uri, updated) = run_local_rename(fixture, "buffer_length");
    let new_code = &updated[&uri];

    assert_eq!(
        new_code,
        "fn calc() -> usize {\n    let buffer_length: usize = 1024;\n    buffer_length * 2\n}\n"
    );
    syn::parse_file(new_code).expect("Valid Rust");
}

#[test]
fn test_rename_multiple_local_variables_in_scope() {
    let fixture = "fn math() -> i32 {\n    let $0first = 1;\n    let second = 2;\n    let third = first + second;\n    first * third\n}\n";
    let (_res, uri, updated) = run_local_rename(fixture, "num1");
    let new_code = &updated[&uri];

    assert_eq!(
        new_code,
        "fn math() -> i32 {\n    let num1 = 1;\n    let second = 2;\n    let third = num1 + second;\n    num1 * third\n}\n"
    );
    syn::parse_file(new_code).expect("Valid Rust");
}

#[test]
fn test_rename_local_variable_in_loop() {
    let fixture = "fn loop_test() {\n    let $0limit = 10;\n    let mut i = 0;\n    while i < limit {\n        i += 1;\n    }\n}\n";
    let (_res, uri, updated) = run_local_rename(fixture, "max_iterations");
    let new_code = &updated[&uri];

    assert_eq!(
        new_code,
        "fn loop_test() {\n    let max_iterations = 10;\n    let mut i = 0;\n    while i < max_iterations {\n        i += 1;\n    }\n}\n"
    );
    syn::parse_file(new_code).expect("Valid Rust");
}

#[test]
fn test_rename_at_identifier_boundary() {
    let fixture = "fn compute() -> i32 {\n    let val$0 = 10;\n    val * 2\n}\n";
    let (_res, uri, updated) = run_local_rename(fixture, "multiplier");
    let new_code = &updated[&uri];

    assert_eq!(
        new_code,
        "fn compute() -> i32 {\n    let multiplier = 10;\n    multiplier * 2\n}\n"
    );
    syn::parse_file(new_code).expect("Valid Rust");
}

#[test]
fn test_rename_invalid_name_rejected() {
    let fixture = "fn test() {\n    let $0x = 1;\n    let _ = x;\n}\n";
    let res = run_local_rename_result(fixture, "123invalid");
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().code, -32803);
}

#[test]
fn test_rename_keyword_rejected() {
    let fixture = "fn test() {\n    let $0x = 1;\n    let _ = x;\n}\n";
    let res = run_local_rename_result(fixture, "match");
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().code, -32803);
}

#[test]
fn test_rename_commutativity_property() {
    let fixture = "fn comm() -> i32 {\n    let $0original = 100;\n    original * 2\n}\n";
    let (_res1, uri1, step1) = run_local_rename(fixture, "renamed");
    let step1_code = &step1[&uri1];
    assert_eq!(
        step1_code,
        "fn comm() -> i32 {\n    let renamed = 100;\n    renamed * 2\n}\n"
    );

    // Step 2: rename back
    let fixture2 = step1_code.replace("let renamed", "let $0renamed");
    let (_res2, uri2, step2) = run_local_rename(&fixture2, "original");
    let step2_code = &step2[&uri2];

    let original_clean = parse_fixture(fixture).clean_text;
    assert_eq!(*step2_code, original_clean);
}
