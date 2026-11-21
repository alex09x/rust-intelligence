/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `Extract into function`: where offered, what it writes, and that the result compiles.

use super::*;
use crate::lsp::editing::tests::code_action_assists_tests::assert_rustc_compiles;

/// `doc` with the expression between the two `$0` extracted, the type of an unannotated `let` given by `ty`.
fn extracted(doc: &str, ty: Option<&str>) -> Option<String> {
    let start = doc.find("$0").unwrap();
    let doc = doc.replacen("$0", "", 1);
    let end = doc.find("$0").unwrap();
    let doc = doc.replacen("$0", "", 1);
    let mut out = doc.clone();
    let mut edits = extract_edits(&doc, start..end, &|_| ty.map(str::to_string))?;
    edits.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
    for (r, text) in edits {
        out.replace_range(r, &text);
    }
    Some(out)
}

/// prod-code, m=20226: rust-analyzer offers `extract_function` on the selection `size * scale`.
#[test]
fn test_an_expression_becomes_a_function_of_the_locals_it_reads() {
    let doc = "pub fn scaled(size: u32, scale: u32) -> u32 {\n    let area = $0size * scale$0;\n    area + 1\n}\n";
    let got = extracted(doc, Some("u32")).unwrap();
    assert_eq!(
        got,
        "pub fn scaled(size: u32, scale: u32) -> u32 {\n    let area = fun_name(size, scale);\n    area + 1\n}\n\nfn fun_name(size: u32, scale: u32) -> u32 {\n    size * scale\n}\n"
    );
    assert_rustc_compiles(&got, "Extract into function");
    // The tail, a `let` read, a constant left as it is, a method of an impl.
    let doc = "const K: u64 = 3;\npub struct S;\nimpl S {\n    pub fn f(&self, a: u64) -> u64 {\n        let b = a + 1;\n        $0b * K + a$0\n    }\n}\n";
    let got = extracted(doc, Some("u64")).unwrap();
    assert!(got.contains("        fun_name(b, a)\n") && got.contains("}\n\nfn fun_name(b: u64, a: u64) -> u64 {\n    b * K + a\n}"), "{got}");
    assert_rustc_compiles(&got, "Extract into function");
    let doc = "pub fn g(s: &str) -> usize {\n    $0s.len() * 2$0\n}\n";
    assert_rustc_compiles(&extracted(doc, None).unwrap(), "Extract into function");
    // A raw name keeps its spelling in the signature.
    let got = extracted("pub fn h(r#type: u8) -> u8 {\n    $0r#type + 1$0\n}\n", None).unwrap();
    assert!(got.contains("fn fun_name(r#type: u8) -> u8") && got.contains("fun_name(r#type)"), "{got}");
    assert_rustc_compiles(&got, "Extract into function");
}

#[test]
fn test_an_expression_is_not_extracted_where_the_result_would_not_compile() {
    for (doc, ty) in [
        // A local passed by value that is not `Copy`, read after.
        ("pub fn f(s: String) -> usize {\n    let n = $0s.len() + s.capacity()$0;\n    s.len() + n\n}\n", Some("usize")),
        // No known type for the value.
        ("pub fn f(a: u32) {\n    println!(\"{}\", $0a + 1$0);\n}\n", None),
        // Leaves the function, names `self`, a macro's arguments.
        ("pub fn f(a: Option<u32>) -> Option<u32> {\n    let b = $0a? + 1$0;\n    Some(b)\n}\n", Some("u32")),
        ("pub struct S(u32);\nimpl S {\n    pub fn f(&self) -> u32 {\n        $0self.0 + 1$0\n    }\n}\n", None),
        ("pub fn f(a: u32) -> String {\n    $0format!(\"{a}\")$0\n}\n", None),
        // Generic.
        ("pub fn f<T: Copy>(a: T) -> T {\n    let b = $0a$0;\n    b\n}\n", Some("T")),
        // A borrow of a local (it would borrow the copy passed), a mutable local (changed through `&mut self` on the
        // copy).
        ("pub fn f(a: u32) -> u32 {\n    let r = $0*&a + 1$0;\n    r\n}\n", Some("u32")),
        ("use std::ops::AddAssign;\npub fn f(mut a: u32) -> u32 {\n    let r = $0{ a.add_assign(1); a }$0;\n    r + a\n}\n", Some("u32")),
        ("pub fn f(a: u32) -> u32 {\n    let mut b = a;\n    let r = $0b.wrapping_add(1)$0;\n    b += 1;\n    r + b\n}\n", Some("u32")),
        // Not a whole expression.
        ("pub fn f(a: u32, b: u32) -> u32 {\n    let c = $0a * b + $01;\n    c\n}\n", Some("u32")),
    ] {
        assert_eq!(extracted(doc, ty), None, "{doc}");
    }
}
