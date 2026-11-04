/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Folding ranges in rust-analyzer's shape.

use super::collect_folding_ranges;

fn spans(src: &str) -> Vec<(u64, u64, String)> {
    let v = collect_folding_ranges(src);
    let mut out: Vec<_> =
        v.as_array().unwrap().iter().map(|r| (r["startLine"].as_u64().unwrap(), r["endLine"].as_u64().unwrap(), r["kind"].as_str().unwrap().to_string())).collect();
    out.sort();
    out
}

fn region(s: u64, e: u64) -> (u64, u64, String) {
    (s, e, "region".to_string())
}

#[test]
fn test_multi_line_header_folds_parameters_function_and_body() {
    let src = "fn f(\n    a: u8,\n) -> u8\nwhere\n    u8: Copy,\n{\n    a\n}\n";
    assert_eq!(spans(src), [region(0, 2), region(0, 7), region(3, 4), region(5, 7)]);
}

#[test]
fn test_if_else_chain_folds_whole_chain_and_each_block() {
    let src = "fn f(x: bool) {\n    if x {\n        1;\n    } else {\n        2;\n    }\n}\n";
    assert_eq!(spans(src), [region(0, 6), region(1, 3), region(1, 5), region(3, 5)]);
}

#[test]
fn test_multi_line_statement_folds() {
    let src = "fn f() {\n    Builder::new()\n        .a()\n        .b();\n}\n";
    assert_eq!(spans(src), [region(0, 4), region(1, 3)]);
}

#[test]
fn test_block_under_attribute_folds_from_attribute() {
    let src = "fn f() {\n    #[cfg(x)]\n    {\n        g();\n    }\n}\n";
    assert_eq!(spans(src), [region(0, 5), region(1, 4)]);
}

#[test]
fn test_runs_of_comments_imports_mods_and_consts() {
    let src = "// a\n// b\n/// doc\nuse a::B;\nuse c::D;\n\nmod x;\nmod y;\npub mod z;\nconst A: u8 = 1;\nconst B: u8 = 2;\n";
    let got = spans(src);
    assert!(got.contains(&(0, 1, "comment".to_string())), "{got:?}");
    assert!(got.contains(&(3, 4, "imports".to_string())), "{got:?}");
    assert!(got.contains(&region(6, 7)), "mod x; mod y; group, pub mod excluded: {got:?}");
    assert!(got.contains(&region(9, 10)), "const group: {got:?}");
    assert_eq!(got.len(), 4, "{got:?}");
}

#[test]
fn test_region_markers_fold_and_leave_comment_runs() {
    let src = "// region: setup\nfn a() {}\nfn b() {}\n// endregion\n";
    assert_eq!(spans(src), [region(0, 3)]);
}

#[test]
fn test_multibyte_text_and_trailing_keyword_do_not_panic() {
    // A brace-free string with CJK text, a comment ending in a multibyte char, and a file ending right after `}`.
    let src = "fn f() {\n    let s = \"标准 {\";\n    // 注释🦀\n    // 第二行🦀\n}";
    let got = spans(src);
    assert!(got.contains(&region(0, 4)), "{got:?}");
    assert!(got.contains(&(2, 3, "comment".to_string())), "{got:?}");
    let v = collect_folding_ranges(src);
    let comment = v.as_array().unwrap().iter().find(|r| r["kind"] == "comment").unwrap();
    assert_eq!(comment["endCharacter"], "    // 第二行🦀".encode_utf16().count());
}
