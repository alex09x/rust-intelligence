/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Lexical token rules that follow rust-analyzer: macro bangs, masked literals, attributes, doc comments and files
//! outside every crate.

use super::tokens::{
    RawToken, TOKEN_TYPE_ENUM, TOKEN_TYPE_FUNCTION, TOKEN_TYPE_INTERFACE, TOKEN_TYPE_MACRO, TOKEN_TYPE_METHOD, TOKEN_TYPE_PROPERTY, TOKEN_TYPE_STRUCT, TOKEN_TYPE_TYPE,
    TOKEN_TYPE_TYPE_PARAMETER, TOKEN_TYPE_VARIABLE,
};
use super::{tokens_attr, tokens_detached, tokens_doc, tokens_scan};

fn spans(tokens: &[RawToken]) -> Vec<(u32, u32, u32, u32)> {
    let mut v: Vec<_> = tokens.iter().map(|t| (t.line, t.col, t.length, t.token_type)).collect();
    v.sort();
    v
}

fn scan(text: &str) -> Vec<RawToken> {
    let mut tokens = Vec::new();
    tokens_scan::scan_source_tokens(text, false, true, &mut tokens);
    tokens
}

#[test]
fn test_macro_call_bang_is_its_own_macro_token() {
    let got = spans(&scan("fn f() { println!(\"x\"); }\n"));
    assert!(got.contains(&(0, 9, 7, TOKEN_TYPE_MACRO)), "{got:?}");
    assert!(got.contains(&(0, 16, 1, TOKEN_TYPE_MACRO)), "{got:?}");
}

#[test]
fn test_imported_macros_and_turbofish_methods() {
    // A file the index does not cover: every identifier gets a lexical type.
    let mut tokens = Vec::new();
    tokens_scan::scan_source_tokens("use clap::{arg, command};\nfn f() { let m = command!().get_one::<String>(\"x\"); arg!(<M>); }\n", false, false, &mut tokens);
    let got = spans(&tokens);
    assert!(got.contains(&(0, 11, 3, TOKEN_TYPE_MACRO)), "arg imported as a macro: {got:?}");
    assert!(got.contains(&(0, 16, 7, TOKEN_TYPE_MACRO)), "command imported as a macro: {got:?}");
    assert!(got.contains(&(1, 28, 7, TOKEN_TYPE_METHOD)), "get_one::<String>: {got:?}");
}

#[test]
fn test_words_in_multi_line_raw_strings_are_not_code() {
    let got = spans(&scan("fn f() {\n    check(r#\"\nfn foo() { Bar::new() }\n\"#);\n}\n"));
    assert!(got.iter().all(|t| t.0 != 2), "no token inside the raw string: {got:?}");
}

#[test]
fn test_std_traits_enums_type_params_and_assoc_types() {
    let got = spans(&scan("fn f<T: Clone>(x: Option<T>) -> Self::Output {}\n"));
    assert!(got.contains(&(0, 8, 5, TOKEN_TYPE_INTERFACE)), "Clone: {got:?}");
    assert!(got.contains(&(0, 18, 6, TOKEN_TYPE_ENUM)), "Option: {got:?}");
    assert!(got.contains(&(0, 25, 1, TOKEN_TYPE_TYPE_PARAMETER)), "T: {got:?}");
    assert!(got.contains(&(0, 38, 6, TOKEN_TYPE_TYPE)), "Self::Output: {got:?}");
}

#[test]
fn test_attribute_contents_dropped_and_derive_arguments_are_macros() {
    let text = "#![cfg_attr(not(test), no_std)]\n#[derive(Debug, Serialize)]\nstruct S;\n";
    let mut tokens = scan(text);
    tokens_attr::apply_attribute_rules(text, &mut tokens);
    let got = spans(&tokens);
    assert!(got.iter().all(|t| t.0 != 0), "nothing inside #![cfg_attr(...)]: {got:?}");
    assert!(got.contains(&(1, 9, 5, TOKEN_TYPE_MACRO)), "Debug: {got:?}");
    assert!(got.contains(&(1, 16, 9, TOKEN_TYPE_MACRO)), "Serialize: {got:?}");
    assert!(!got.iter().any(|t| t.0 == 1 && t.1 == 2), "`derive` itself: {got:?}");
}

#[test]
fn test_doc_examples_in_rust_fences_are_scanned_and_other_fences_are_not() {
    let text = "/// Example:\n/// ```\n/// let v = Vec::new();\n/// ```\n/// ```text\n/// Foo::bar()\n/// ```\nfn f() {}\n";
    let mut tokens = Vec::new();
    tokens_doc::scan_doc_comments(text, None, &mut tokens);
    let got = spans(&tokens);
    assert!(got.contains(&(2, 8, 1, TOKEN_TYPE_VARIABLE)), "v: {got:?}");
    assert!(got.contains(&(2, 12, 3, TOKEN_TYPE_STRUCT)), "Vec: {got:?}");
    assert!(got.iter().all(|t| t.0 != 5), "a `text` fence is not code: {got:?}");
}

#[test]
fn test_a_nested_block_comment_does_not_end_the_doc_block() {
    // `src/**/foo.rs` opens and closes a nested comment: the `/*! … */` block goes on to its example.
    let text = "/*!\n* `src/**/foo.rs`\n\n```\nlet v = Vec::new();\n```\n*/\nfn f() {}\n";
    let mut tokens = Vec::new();
    tokens_doc::scan_doc_comments(text, None, &mut tokens);
    let got = spans(&tokens);
    assert!(got.contains(&(4, 4, 1, TOKEN_TYPE_VARIABLE)), "v: {got:?}");
    assert!(got.contains(&(4, 8, 3, TOKEN_TYPE_STRUCT)), "Vec: {got:?}");
    assert!(got.iter().all(|t| t.0 != 7), "code after the block is no doc: {got:?}");
}

#[test]
fn test_doc_blocks_span_their_comment_lines() {
    // A `//!` run, a `/*! … */` block with a nested comment, and a `///` run; a plain `//` line is no doc.
    let text = "//! Crate docs.\n//! ```\n//! let a = 1;\n//! ```\n\n// plain\n/*!\nsrc/**/x.rs\n*/\nfn f() {}\n/// Item docs.\nfn g() {}\n";
    assert_eq!(tokens_doc::doc_block_spans(text), vec![(0, 3), (6, 8), (10, 10)]);
}

#[test]
fn test_detached_file_types_definitions_and_bindings_only() {
    let text = "use other::Thing;\nstruct S {\n    a: u8,\n}\nfn main() {\n    let x = Thing::new();\n    let f = |y: u8| y;\n}\n";
    let mut tokens = Vec::new();
    tokens_detached::detached_definition_tokens(text, &mut tokens);
    let got = spans(&tokens);
    assert!(got.contains(&(1, 7, 1, TOKEN_TYPE_STRUCT)), "{got:?}");
    assert!(got.contains(&(2, 4, 1, TOKEN_TYPE_PROPERTY)), "{got:?}");
    assert!(got.contains(&(4, 3, 4, TOKEN_TYPE_FUNCTION)), "{got:?}");
    assert!(got.contains(&(5, 8, 1, TOKEN_TYPE_VARIABLE)), "{got:?}");
    assert!(got.contains(&(6, 13, 1, TOKEN_TYPE_VARIABLE)), "closure parameter: {got:?}");
    assert!(got.iter().all(|t| t.0 != 0), "the unresolved import has no token: {got:?}");
    assert!(!got.iter().any(|t| t.0 == 5 && t.1 >= 12), "uses have no token: {got:?}");
}

#[test]
fn test_outside_every_crate_follows_cargo_layout() {
    let dir = std::env::temp_dir().join(format!("ridx_tok_layout_{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src/inner")).unwrap();
    std::fs::create_dir_all(dir.join("tests/ui")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"p\"\n").unwrap();
    let at = |rel: &str| dir.join(rel).to_string_lossy().into_owned();
    assert!(!tokens_detached::outside_every_crate(&at("build.rs")));
    assert!(!tokens_detached::outside_every_crate(&at("src/inner/gated.rs")));
    assert!(tokens_detached::outside_every_crate(&at("tests/ui/case.rs")));
    assert!(!tokens_detached::outside_every_crate(&at("examples/demo.rs")), "auto-discovered example");
    assert!(!tokens_detached::outside_every_crate(&at("tests/suite/main.rs")), "auto-discovered test");
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"p\"\n[[example]]\nname = \"t\"\npath = \"examples/tutorial/01.rs\"\n").unwrap();
    assert!(!tokens_detached::outside_every_crate(&at("examples/tutorial/01.rs")), "manifest-declared example");
    // A nested package with its own `[workspace]` (a fuzz crate) is not loaded with the outer workspace.
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"p\"\n[workspace]\nmembers = []\n").unwrap();
    std::fs::create_dir_all(dir.join("fuzz/fuzz_targets")).unwrap();
    std::fs::write(dir.join("fuzz/Cargo.toml"), "[package]\nname = \"f\"\n[workspace]\n[[bin]]\nname = \"a\"\npath = \"fuzz_targets/a.rs\"\n").unwrap();
    assert!(tokens_detached::outside_every_crate(&at("fuzz/fuzz_targets/a.rs")), "separate nested workspace");
    assert!(!tokens_detached::outside_every_crate(&at("src/inner/gated.rs")), "the outer workspace root package stays in");
    assert!(tokens_detached::gated_off_by_inner_cfg("//! Docs\n#![cfg(all(tokio_unstable, feature = \"x\"))]\nfn f() {}\n"));
    assert!(!tokens_detached::gated_off_by_inner_cfg("#![allow(dead_code)]\nfn f() {}\n"));
    let _ = std::fs::remove_dir_all(&dir);
}
