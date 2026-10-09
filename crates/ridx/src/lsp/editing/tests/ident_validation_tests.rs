/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Exhaustive tests for Rust identifier validation, keyword detection,
//! and declaration column resolution across all DefKinds.

use crate::lsp::editing::ident::*;
use ridx_format::DefKind;

#[test]
fn test_valid_ascii_identifiers() {
    let valid_cases = [
        "foo",
        "bar_baz",
        "_private",
        "__double_under",
        "camelCase",
        "PascalCase",
        "SCREAMING_SNAKE",
        "x",
        "_1",
        "foo123",
        "var_with_numbers_123_and_more",
    ];
    for case in valid_cases {
        assert!(is_valid_rust_ident(case), "Expected valid: '{}'", case);
    }
}

#[test]
fn test_invalid_ascii_identifiers() {
    let invalid_cases = [
        "", "1foo", "123", "-foo", "foo-bar", "foo.bar", "foo::bar", "foo!", "foo?", "foo bar",
        "foo\tbar", "foo\nbar", "@foo", "$foo", "#foo",
    ];
    for case in invalid_cases {
        assert!(!is_valid_rust_ident(case), "Expected invalid: '{}'", case);
    }
}

#[test]
fn test_strict_keywords_rejected_as_idents() {
    let keywords = [
        "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn",
        "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
        "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe",
        "use", "where", "while", "async", "await", "dyn", "abstract", "become", "box", "do",
        "final", "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "try",
    ];
    for kw in keywords {
        assert!(is_rust_keyword(kw), "Expected keyword: '{}'", kw);
        assert!(
            !is_valid_rust_ident(kw),
            "Keyword must not be valid ident: '{}'",
            kw
        );
    }
}

#[test]
fn test_non_keywords_allowed_as_idents() {
    let non_keywords = [
        "match_var",
        "fn_ptr",
        "let_binding",
        "default",
        "union",
        "gen",
        "raw",
        "safe",
        "pin",
        "drop",
    ];
    for name in non_keywords {
        assert!(!is_rust_keyword(name), "Expected non-keyword: '{}'", name);
        assert!(
            is_valid_rust_ident(name),
            "Expected valid ident: '{}'",
            name
        );
    }
}

#[test]
fn test_raw_identifiers() {
    // In Rust, raw identifiers allow keywords like r#match, r#fn, r#type
    let raw_cases = ["r#match", "r#fn", "r#type", "r#async", "r#struct"];
    for raw in raw_cases {
        let clean = raw.strip_prefix("r#").unwrap();
        assert!(is_rust_keyword(clean));
        assert!(
            is_valid_rust_ident(raw),
            "Raw ident should be valid: {}",
            raw
        );
    }
    let forbidden_raw = ["r#crate", "r#self", "r#super", "r#Self"];
    for raw in forbidden_raw {
        assert!(!is_valid_rust_ident(raw), "Forbidden raw ident: {}", raw);
    }
}

#[test]
fn test_unicode_identifiers() {
    let unicode_cases = [
        "grüßen",
        "smörgås",
        "variable_δ",
        "alpha_α",
        "β_beta",
        "日本語",
    ];
    for case in unicode_cases {
        assert!(
            is_valid_rust_ident(case),
            "Unicode ident should be valid: '{}'",
            case
        );
    }
}

#[test]
fn test_declaration_ident_col_fn() {
    assert_eq!(
        find_declaration_ident_col("fn run() {}", "run", DefKind::Fn),
        Some(4)
    );
    assert_eq!(
        find_declaration_ident_col("pub fn execute() {}", "execute", DefKind::Fn),
        Some(8)
    );
    assert_eq!(
        find_declaration_ident_col("pub async fn fetch() {}", "fetch", DefKind::Fn),
        Some(14)
    );
    assert_eq!(
        find_declaration_ident_col("pub const unsafe fn init() {}", "init", DefKind::Fn),
        Some(21)
    );
    assert_eq!(
        find_declaration_ident_col("pub extern \"C\" fn c_call() {}", "c_call", DefKind::Fn),
        Some(19)
    );
}

#[test]
fn test_declaration_ident_col_struct_and_enum() {
    assert_eq!(
        find_declaration_ident_col("struct Point { x: i32 }", "Point", DefKind::Struct),
        Some(8)
    );
    assert_eq!(
        find_declaration_ident_col("pub struct Buffer;", "Buffer", DefKind::Struct),
        Some(12)
    );
    assert_eq!(
        find_declaration_ident_col("enum State { Active }", "State", DefKind::Enum),
        Some(6)
    );
    assert_eq!(
        find_declaration_ident_col("pub enum Option<T> {", "Option", DefKind::Enum),
        Some(10)
    );
}

#[test]
fn test_declaration_ident_col_trait_and_type() {
    assert_eq!(
        find_declaration_ident_col("trait Worker {}", "Worker", DefKind::Trait),
        Some(7)
    );
    assert_eq!(
        find_declaration_ident_col("pub unsafe trait SendSync {}", "SendSync", DefKind::Trait),
        Some(18)
    );
    assert_eq!(
        find_declaration_ident_col(
            "type Result<T> = std::result::Result<T, ()>;",
            "Result",
            DefKind::TyAlias
        ),
        Some(6)
    );
    assert_eq!(
        find_declaration_ident_col("pub type NodeId = u64;", "NodeId", DefKind::TyAlias),
        Some(10)
    );
}

#[test]
fn test_declaration_ident_col_const_and_static() {
    assert_eq!(
        find_declaration_ident_col("const MAX_BUF: usize = 1024;", "MAX_BUF", DefKind::Const),
        Some(7)
    );
    assert_eq!(
        find_declaration_ident_col("pub const TIMEOUT: u64 = 5;", "TIMEOUT", DefKind::Const),
        Some(11)
    );
    assert_eq!(
        find_declaration_ident_col("static COUNTER: AtomicU64 = 0;", "COUNTER", DefKind::Static),
        Some(8)
    );
    assert_eq!(
        find_declaration_ident_col("pub static mut GLOBAL: i32 = 0;", "GLOBAL", DefKind::Static),
        Some(16)
    );
}

#[test]
fn test_declaration_ident_col_macro_and_mod() {
    assert_eq!(
        find_declaration_ident_col("macro_rules! my_macro {", "my_macro", DefKind::Macro),
        Some(14)
    );
    assert_eq!(
        find_declaration_ident_col("mod parser;", "parser", DefKind::Mod),
        Some(5)
    );
    assert_eq!(
        find_declaration_ident_col("pub mod tests {", "tests", DefKind::Mod),
        Some(9)
    );
}

#[test]
fn test_declaration_ident_col_skips_comments_and_strings() {
    // Identifier appears in comment or string before actual declaration
    let line_with_comment = "/* fn helper */ fn helper() {}";
    assert_eq!(
        find_declaration_ident_col(line_with_comment, "helper", DefKind::Fn),
        Some(20)
    );

    let line_with_string = "const MSG: &str = \"struct Foo\"; struct Foo;";
    assert_eq!(
        find_declaration_ident_col(line_with_string, "Foo", DefKind::Struct),
        Some(40)
    );
}

#[test]
fn test_declaration_ident_col_not_found() {
    assert_eq!(
        find_declaration_ident_col("fn other() {}", "target", DefKind::Fn),
        None
    );
    assert_eq!(
        find_declaration_ident_col("// fn target() {}", "target", DefKind::Fn),
        None
    );
}
