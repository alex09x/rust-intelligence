/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `Promote local to constant`: where offered, what it writes, and that the result compiles.

use super::*;
use crate::lsp::editing::tests::code_action_assists_tests::assert_rustc_compiles;

/// `doc` with the promotion at the first `$0` applied.
fn promoted(doc: &str, _: Option<&str>) -> Option<String> {
    let offset = doc.find("$0").unwrap();
    let doc = doc.replacen("$0", "", 1);
    let mut out = doc.clone();
    for (r, text) in promote_edits(&doc, offset)?.into_iter().rev() {
        out.replace_range(r, &text);
    }
    Some(out)
}

#[test]
fn test_a_constant_binding_becomes_a_constant() {
    let doc = "pub fn f(v: &[u8]) -> bool {\n    let max$0_len: usize = 32;\n    let ok = v.len() < max_len;\n    ok && max_len > 1\n}\n";
    let got = promoted(doc, None).unwrap();
    assert_eq!(got, "pub fn f(v: &[u8]) -> bool {\n    const MAX_LEN: usize = 32;\n    let ok = v.len() < MAX_LEN;\n    ok && MAX_LEN > 1\n}\n");
    assert_rustc_compiles(&got, "Promote local to constant");
    let got = promoted("pub fn g() -> u64 {\n    let n$0 = 8u64;\n    n\n}\n", None).unwrap();
    assert_eq!(got, "pub fn g() -> u64 {\n    const N: u64 = 8u64;\n    N\n}\n", "the type the literal spells");
    assert_rustc_compiles(&got, "Promote local to constant");
    let got = promoted("pub fn h() -> String {\n    let greeting$0 = \"hi\";\n    greeting.to_string()\n}\n", None).unwrap();
    assert!(got.contains("const GREETING: &str = \"hi\";") && got.contains("GREETING.to_string()"), "{got}");
    assert_rustc_compiles(&got, "Promote local to constant");
    // Review of 63f24322: a field shorthand keeps its field; a function of the block named so is not the local.
    let got = promoted("pub struct S { pub x: i32 }\npub fn s() -> S {\n    let x$0: i32 = 1;\n    S { x }\n}\n", None).unwrap();
    assert!(got.contains("S { x: X }"), "{got}");
    assert_rustc_compiles(&got, "Promote local to constant");
    // Review of 36349abf: with an attribute before it too.
    let got = promoted("pub struct S { pub x: i32 }\npub fn s() -> S {\n    let x$0: i32 = 1;\n    S { #[cfg(all())] x }\n}\n", None).unwrap();
    assert!(got.contains("S { #[cfg(all())] x: X }"), "{got}");
    assert_rustc_compiles(&got, "Promote local to constant");
    let got = promoted("pub fn v() -> i32 {\n    let value$0: i32 = 1;\n    fn value() -> i32 {\n        2\n    }\n    value\n}\n", None).unwrap();
    assert!(got.contains("const VALUE: i32 = 1;") && got.ends_with("    VALUE\n}\n"), "{got}");
    assert_rustc_compiles(&got, "Promote local to constant");
}

/// prod-code, m=20226: rust-analyzer promotes `let area = size * scale;`, which reads parameters: no constant can.
#[test]
fn test_a_binding_no_constant_can_hold_is_not_promoted() {
    for (doc, ty) in [
        ("pub fn scaled(size: u32, scale: u32) -> u32 {\n    let $0area = size * scale;\n    area + 1\n}\n", Some("u32")),
        ("pub fn f() -> usize {\n    let n$0 = vec![1].len();\n    n\n}\n", Some("usize")),
        ("pub fn f() -> u32 {\n    let mut n$0 = 1;\n    n += 1;\n    n\n}\n", Some("u32")),
        // Formatted by name, which a rename does not see.
        ("pub fn f() -> String {\n    let n$0 = 1;\n    format!(\"{n}\")\n}\n", Some("i32")),
        // The name is taken.
        ("const N: u8 = 2;\npub fn f() -> u8 {\n    let n$0 = 1;\n    n + N\n}\n", Some("u8")),
        // No type to write.
        ("pub fn f() -> u8 {\n    let n$0 = 1;\n    n\n}\n", None),
        // Review of 63f24322: a name taken in raw spelling, a placeholder or generic type, an operator no constant
        // evaluates.
        ("const r#N: u8 = 2;\npub fn f() -> u8 {\n    let n$0 = 1u8;\n    n + N\n}\n", Some("u8")),
        ("pub fn f() -> i32 {\n    let x$0: _ = 1;\n    x\n}\n", None),
        ("pub fn f<T>() -> usize {\n    let empty$0: [T; 0] = [];\n    empty.len()\n}\n", None),
        ("pub fn f() -> bool {\n    let same$0 = \"a\" == \"b\";\n    same\n}\n", Some("bool")),
        ("pub fn f() -> f64 {\n    let x$0 = 1.5 * 2.0;\n    x\n}\n", Some("f64")),
        // Review of 36349abf: a comparison of tuples, an impl's `Self`.
        ("pub fn f() -> bool {\n    let same$0 = (1u8, 2u8) == (1u8, 2u8);\n    same\n}\n", Some("bool")),
        ("pub struct S;\nimpl S {\n    pub fn f() -> usize {\n        let empty$0: [Self; 0] = [];\n        empty.len()\n    }\n}\n", None),
        // Review of 03933e37: an operator, which a constant evaluates at compile time (a division by zero or an
        // overflow then fails to compile); a type only inferred; a primitive's name the file takes.
        ("pub fn f(c: bool) -> u8 {\n    let x$0 = 1u8 / 0u8;\n    if c { x } else { 0 }\n}\n", None),
        ("pub fn f() -> usize {\n    let n$0: usize = 4 * 8;\n    n\n}\n", None),
        ("pub fn f() -> f64 {\n    let x$0 = 1.5;\n    x\n}\n", None),
        ("type u8 = i64;\npub fn f() -> u8 {\n    let n$0 = 1u8;\n    n\n}\n", None),
        // Review of b854363f: a glob that may name the literal's type, a token in a macro's arguments (a pattern's new
        // binding there), a name the new one is in normal form, a raw borrow of the place.
        ("mod t {\n    pub type u8 = String;\n}\nuse t::*;\npub fn f() -> bool {\n    let n$0 = 1u8;\n    n > 0\n}\n", None),
        ("pub fn f() -> bool {\n    let x$0: u8 = 1;\n    matches!(2u8, x)\n}\n", None),
        ("const \u{212a}: i32 = 9;\npub fn f() -> i32 {\n    let k$0: i32 = 1;\n    k + K\n}\n", None),
        ("pub fn f() -> *const u8 {\n    let x$0: u8 = 1;\n    &raw const x\n}\n", None),
    ] {
        assert_eq!(promoted(doc, ty), None, "{doc}");
    }
}

/// Review of 7dca0b11: a macro that may import `u8` keeps the literal's own type unwritten.
#[test]
fn test_a_macro_that_may_import_keeps_the_type_unwritten() {
    let doc = "mod t {\n    pub type u8 = String;\n}\nmacro_rules! import {\n    () => {\n        use t::*;\n    };\n}\nimport!();\npub fn f() -> bool {\n    let n$0 = 1u8;\n    n > 0\n}\n";
    assert_eq!(promoted(doc, None), None);
}
