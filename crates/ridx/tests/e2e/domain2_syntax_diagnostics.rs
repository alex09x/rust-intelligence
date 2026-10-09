/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 2: Unit tests for real-time syntax diagnostics, delimiter balancing,
//! raw string scanning, comment escaping, and let-binding type mismatch detection.

use ridx::lsp::presentation::diagnostics::{
    handle_document_diagnostic_text, publish_syntax_diagnostics, reconcile_diagnostics,
};
use ridx::overlay::OverlayManager;
use serde_json::json;

#[test]
fn test_syntax_clean_code_zero_diagnostics() {
    let code = r#"
pub fn add(a: i32, b: i32) -> i32 {
    let result = a + b;
    result
}
"#;
    let res = handle_document_diagnostic_text(code);
    let items = res["items"].as_array().expect("items array");
    assert!(
        items.is_empty(),
        "Valid code must produce 0 syntax diagnostics"
    );
}

#[test]
fn test_syntax_unclosed_brace_at_eof() {
    let code = "fn broken() {\n    let a = 1;\n";
    let res = handle_document_diagnostic_text(code);
    let items = res["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["severity"], 1);
    assert!(items[0]["message"]
        .as_str()
        .unwrap()
        .contains("unclosed delimiter '{'"));
    assert_eq!(items[0]["range"]["start"]["line"], 0);
}

#[test]
fn test_syntax_unclosed_parenthesis_and_bracket() {
    let code1 = "fn func(a: i32, b: i32 {\n    let x = 1;\n}\n";
    let res1 = handle_document_diagnostic_text(code1);
    let items1 = res1["items"].as_array().unwrap();
    assert!(!items1.is_empty());
    assert!(items1
        .iter()
        .any(|d| d["message"].as_str().unwrap().contains("delimiter")));

    let code2 = "let arr = [1, 2, 3;\n";
    let res2 = handle_document_diagnostic_text(code2);
    let items2 = res2["items"].as_array().unwrap();
    assert!(!items2.is_empty());
    assert!(items2.iter().any(|d| d["message"]
        .as_str()
        .unwrap()
        .contains("unclosed delimiter '['")));
}

#[test]
fn test_syntax_unmatched_closing_delimiters() {
    let code = "pub fn test() }\n";
    let res = handle_document_diagnostic_text(code);
    let items = res["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert!(items[0]["message"]
        .as_str()
        .unwrap()
        .contains("unmatched closing delimiter '}'"));

    let code_paren = "let x = 1);\n";
    let res_paren = handle_document_diagnostic_text(code_paren);
    let items_paren = res_paren["items"].as_array().unwrap();
    assert_eq!(items_paren.len(), 1);
    assert!(items_paren[0]["message"]
        .as_str()
        .unwrap()
        .contains("unmatched closing delimiter ')'"));

    let code_bracket = "let y = 1];\n";
    let res_bracket = handle_document_diagnostic_text(code_bracket);
    let items_bracket = res_bracket["items"].as_array().unwrap();
    assert_eq!(items_bracket.len(), 1);
    assert!(items_bracket[0]["message"]
        .as_str()
        .unwrap()
        .contains("unmatched closing delimiter ']'"));
}

#[test]
fn test_syntax_interleaved_mismatched_delimiters() {
    // Opening '(' followed by '{', but closing ')' before '}'
    let code = "fn test() { ( }\n";
    let res = handle_document_diagnostic_text(code);
    let items = res["items"].as_array().unwrap();
    assert!(!items.is_empty());
}

#[test]
fn test_syntax_unclosed_string_literal() {
    let code = "let s = \"hello world\nlet next = 1;\n";
    let res = handle_document_diagnostic_text(code);
    let items = res["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert!(items[0]["message"]
        .as_str()
        .unwrap()
        .contains("unclosed string literal"));
    assert_eq!(items[0]["range"]["start"]["line"], 0);
}

#[test]
fn test_syntax_unclosed_raw_string_single_and_multi_hash() {
    let code1 = "let r = r#\"unclosed raw string;\nlet x = 1;\n";
    let res1 = handle_document_diagnostic_text(code1);
    let items1 = res1["items"].as_array().unwrap();
    assert_eq!(items1.len(), 1);
    assert!(items1[0]["message"]
        .as_str()
        .unwrap()
        .contains("unclosed raw string literal"));

    let code2 = "let r = r###\"three hashes missing end;\n";
    let res2 = handle_document_diagnostic_text(code2);
    let items2 = res2["items"].as_array().unwrap();
    assert_eq!(items2.len(), 1);
    assert!(items2[0]["message"]
        .as_str()
        .unwrap()
        .contains("unclosed raw string literal"));
}

#[test]
fn test_syntax_properly_closed_raw_string_with_internal_quotes() {
    let code = "let r = r#\"contains \"quotes\" and {braces} inside\"#;\n";
    let res = handle_document_diagnostic_text(code);
    let items = res["items"].as_array().unwrap();
    assert!(
        items.is_empty(),
        "Properly closed raw string must not report errors"
    );
}

#[test]
fn test_syntax_comments_ignore_delimiters() {
    let code = r#"
// line comment with { unclosed brace and ( unclosed paren
/* block comment with { [ ( unclosed
   and multiline content
*/
pub fn valid() -> i32 {
    42
}
"#;
    let res = handle_document_diagnostic_text(code);
    let items = res["items"].as_array().unwrap();
    assert!(
        items.is_empty(),
        "Delimiters inside comments must be ignored"
    );
}

#[test]
fn test_syntax_nested_block_comments() {
    let code = r#"
/* outer /* inner block comment */ still outer */
pub fn check() {}
"#;
    let res = handle_document_diagnostic_text(code);
    let items = res["items"].as_array().unwrap();
    assert!(
        items.is_empty(),
        "Nested block comments must be handled cleanly"
    );
}

#[test]
fn test_syntax_delimiters_inside_char_and_string_literals() {
    let code = r#"
pub fn check() {
    let c1 = '{';
    let c2 = '(';
    let s = "{([})]";
}
"#;
    let res = handle_document_diagnostic_text(code);
    let items = res["items"].as_array().unwrap();
    assert!(
        items.is_empty(),
        "Delimiters inside chars and strings must be ignored"
    );
}

#[test]
fn test_syntax_let_type_mismatch_int_expected_str_found() {
    let code = "fn test() {\n    let val: i32 = \"hello world\";\n}\n";
    let res = handle_document_diagnostic_text(code);
    let items = res["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    let diag = &items[0];
    assert_eq!(diag["code"], "E0308");
    assert!(diag["message"]
        .as_str()
        .unwrap()
        .contains("mismatched types"));
    assert!(diag["message"]
        .as_str()
        .unwrap()
        .contains("expected `i32`, found `&str`"));
    assert_eq!(diag["range"]["start"]["line"], 1);
}

#[test]
fn test_syntax_let_type_mismatch_bool_expected_int_found() {
    let code = "fn test() {\n    let flag: bool = 42;\n}\n";
    let res = handle_document_diagnostic_text(code);
    let items = res["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    let diag = &items[0];
    assert_eq!(diag["code"], "E0308");
    assert!(diag["message"]
        .as_str()
        .unwrap()
        .contains("expected `bool`, found `{integer}`"));
}

#[test]
fn test_syntax_let_type_mismatch_str_expected_int_found() {
    let code = "fn test() {\n    let text: &str = 100;\n}\n";
    let res = handle_document_diagnostic_text(code);
    let items = res["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["code"], "E0308");
    assert!(items[0]["message"]
        .as_str()
        .unwrap()
        .contains("expected `&str`, found `{integer}`"));
}

#[test]
fn test_publish_syntax_diagnostics_overlay_lookup() {
    let mut mgr = OverlayManager::new();
    let file = "/tmp/test_syntax_diag_file.rs";
    let uri = format!("file://{file}");
    mgr.open_document(&uri, file, 1, "pub fn unclosed() {\n".to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri }
        }
    });

    let res = publish_syntax_diagnostics(&req, &mgr);
    let items = res["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert!(items[0]["message"]
        .as_str()
        .unwrap()
        .contains("unclosed delimiter '{'"));
}

#[test]
fn test_syntax_empty_and_whitespace_only() {
    let res_empty = handle_document_diagnostic_text("");
    assert!(res_empty["items"].as_array().unwrap().is_empty());

    let res_ws = handle_document_diagnostic_text("   \n\n\t  \r\n   ");
    assert!(res_ws["items"].as_array().unwrap().is_empty());
}

#[test]
fn test_syntax_let_type_mismatch_usize_bool() {
    let code = "fn test() {\n    let len: usize = true;\n}\n";
    let res = handle_document_diagnostic_text(code);
    let items = res["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["code"], "E0308");
    assert!(items[0]["message"]
        .as_str()
        .unwrap()
        .contains("expected `usize`, found `bool`"));
}

#[test]
fn test_syntax_unclosed_block_comment_at_eof() {
    let code = "pub fn foo() {}\n/* unclosed block comment";
    let res = handle_document_diagnostic_text(code);
    // Unclosed block comment is error-tolerated by scanner without crashing
    let items = res["items"].as_array().unwrap();
    assert!(
        items.is_empty()
            || items
                .iter()
                .all(|i| i["severity"].as_u64().unwrap_or(0) >= 1)
    );
}

#[test]
fn test_syntax_and_flycheck_diagnostics_coexistence_and_deduplication() {
    let code = "fn test() {\n    let len: usize = true;\n    let y = 1;\n}\n";
    let syntax_res = handle_document_diagnostic_text(code);
    let heuristic_items = syntax_res["items"].as_array().unwrap();
    assert_eq!(heuristic_items.len(), 1);
    assert_eq!(heuristic_items[0]["source"], "ridx");
    assert_eq!(heuristic_items[0]["code"], "E0308");
    assert_eq!(heuristic_items[0]["range"]["start"]["line"], 1);

    // Simulated flycheck diagnostics from rustc on the same error (line 1, E0308)
    let authoritative_items = vec![
        json!({
            "range": {
                "start": { "line": 1, "character": 21 },
                "end": { "line": 1, "character": 25 }
            },
            "severity": 1,
            "code": "E0308",
            "source": "rustc",
            "message": "mismatched types\nexpected `usize`, found `bool`"
        }),
        json!({
            "range": {
                "start": { "line": 2, "character": 8 },
                "end": { "line": 2, "character": 9 }
            },
            "severity": 2,
            "code": "unused_variables",
            "source": "rustc",
            "message": "unused variable: `y`"
        }),
    ];

    // Reconcile: heuristic E0308 on line 1 is suppressed because rustc authoritative E0308 is present
    let reconciled = reconcile_diagnostics(heuristic_items, &authoritative_items);

    // Exactly 2 diagnostics: both authoritative rustc diagnostics, 0 duplicated E0308 from ridx
    assert_eq!(reconciled.len(), 2);
    assert!(reconciled.iter().all(|d| d["source"] == "rustc"));
    let codes: Vec<&str> = reconciled
        .iter()
        .map(|d| d["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, vec!["E0308", "unused_variables"]);

    // If flycheck has no E0308 (e.g. only unused warning on line 2), heuristic E0308 is retained
    let flycheck_only_unused = vec![json!({
        "range": {
            "start": { "line": 2, "character": 8 },
            "end": { "line": 2, "character": 9 }
        },
        "severity": 2,
        "code": "unused_variables",
        "source": "rustc",
        "message": "unused variable: `y`"
    })];
    let reconciled_with_heuristic = reconcile_diagnostics(heuristic_items, &flycheck_only_unused);
    assert_eq!(reconciled_with_heuristic.len(), 2);
    assert_eq!(reconciled_with_heuristic[0]["source"], "rustc");
    assert_eq!(reconciled_with_heuristic[1]["source"], "ridx");
}

#[test]
fn test_reconcile_diagnostics_preserves_non_overlapping_errors_on_same_line() {
    let heuristic = vec![
        json!({
            "range": { "start": { "line": 5, "character": 12 }, "end": { "line": 5, "character": 15 } },
            "code": "E0308", "source": "ridx"
        }),
        json!({
            "range": { "start": { "line": 5, "character": 30 }, "end": { "line": 5, "character": 31 } },
            "code": "E0308", "source": "ridx"
        }),
    ];
    let authoritative = vec![json!({
        "range": { "start": { "line": 5, "character": 12 }, "end": { "line": 5, "character": 15 } },
        "code": "E0308", "source": "rustc"
    })];
    let merged = reconcile_diagnostics(&heuristic, &authoritative);
    assert_eq!(merged.len(), 2);
    assert_eq!(merged[0]["source"], "rustc");
    assert_eq!(merged[0]["range"]["start"]["character"], 12);
    assert_eq!(merged[1]["source"], "ridx");
    assert_eq!(merged[1]["range"]["start"]["character"], 30);
}
