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
    let doc = "fn width(s: &str) -> usize { s.len() }\npub fn g(s: &str) -> usize {\n    $0width(s) * 2$0\n}\n";
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
        // Review of 531974ed: a name the body declares, a write, a type the new signature cannot hold, a primitive's
        // name taken, `&'a mut`, `ref`, a context the helper would lack.
        ("const N: i32 = 100;\npub fn f() -> i32 {\n    const N: i32 = 3;\n    let r = $0N + 1$0;\n    r\n}\n", Some("i32")),
        ("pub fn f() -> i32 {\n    fn local() -> i32 { 1 }\n    let r = $0local() + 1$0;\n    r\n}\n", Some("i32")),
        ("pub fn f(mut n: i32) -> i32 {\n    let r = $0n += 1$0;\n    n\n}\n", Some("()")),
        ("pub struct S;\nimpl S {\n    pub fn f(a: bool) -> Self {\n        $0if a { S } else { S }$0\n    }\n}\n", None),
        ("pub fn f<'a>(a: &'a str, b: &'a str, c: bool) -> &'a str {\n    $0if c { a } else { b }$0\n}\n", None),
        ("pub fn f(v: &u8) -> u8 {\n    let x: &_ = v;\n    let r = $0*x + 1$0;\n    r\n}\n", Some("u8")),
        ("pub type u32 = String;\npub fn f(v: u32) -> usize {\n    let n = $0v.len() + 1$0;\n    n + v.len()\n}\n", Some("usize")),
        ("pub fn f(x: &'static mut i32) -> i32 {\n    let r = $0*x + 1$0;\n    r + *x\n}\n", Some("i32")),
        ("pub fn f(ref x: i32) -> i32 {\n    let r = $0*x + 1$0;\n    r\n}\n", Some("i32")),
        ("unsafe fn g() -> i32 { 1 }\npub fn f() -> i32 {\n    unsafe {\n        let r = $0g() + 1$0;\n        r\n    }\n}\n", Some("i32")),
        ("#[cfg(test)]\nfn only() -> i32 { 1 }\n#[cfg(test)]\npub fn f() -> i32 {\n    let r = $0only() + 1$0;\n    r\n}\n", Some("i32")),
        ("#[track_caller]\npub fn f(a: u32) -> u32 {\n    let r = $0a.checked_add(1).unwrap()$0;\n    r\n}\n", Some("u32")),
        // Review of 6c5f3f95: a receiver borrowed by the compiler, a `let` assigned later.
        ("pub trait A { fn address(&self) -> usize; }\nimpl A for u8 { fn address(&self) -> usize { self as *const u8 as usize } }\npub fn f(n: u8) -> usize {\n    let r = $0n.address() + 1$0;\n    r\n}\n", Some("usize")),
        ("pub fn f() -> u32 {\n    let x: u32;\n    let r = $0{ x = 1; x }$0;\n    r\n}\n", Some("u32")),
        ("pub fn f(c: bool) -> u32 {\n    let x: u32;\n    if c { x = 1 } else { x = 2 }\n    let r = $0x + 1$0;\n    r\n}\n", Some("u32")),
        // Review of 5852217f: a name in another normal form, a type the body declares, a primitive's name taken raw or
        // by an import, a `cfg` on the statement or spelled raw, an array's length.
        ("const caf\u{e9}: i32 = 100;\npub fn f() -> i32 {\n    const cafe\u{301}: i32 = 3;\n    let r = $0caf\u{e9} + 1$0;\n    r\n}\n", Some("i32")),
        ("pub fn f(v: &u8) -> u8 {\n    type Word = u8;\n    let n: Word = *v;\n    let r = $0n + 1$0;\n    r\n}\n", Some("Word")),
        ("type r#u32 = String;\npub fn f(v: u32) -> usize {\n    let n = $0v.len() + 1$0;\n    n + v.len()\n}\n", Some("usize")),
        ("use std::string::String as u32;\npub fn f(v: u32) -> usize {\n    let n = $0v.len() + 1$0;\n    n\n}\n", Some("usize")),
        ("pub fn f(a: u8) -> u8 {\n    #[cfg(test)]\n    let r = $0a + 1$0;\n    #[cfg(not(test))]\n    let r = a;\n    r\n}\n", Some("u8")),
        ("#[r#cfg(test)]\npub fn f(a: u8) -> u8 {\n    let r = $0a + 1$0;\n    r\n}\n", Some("u8")),
        ("pub fn f() -> usize {\n    let r = [0u8; { $01 + 2$0 }];\n    r.len()\n}\n", None),
        // Review of 56f8c6cd: a receiver in parentheses is borrowed in place too; a comparison of a type an import
        // names as a primitive borrows what is not `Copy`.
        ("pub trait A { fn address(&self) -> usize; }\nimpl A for u8 { fn address(&self) -> usize { self as *const u8 as usize } }\npub fn f(n: u8) -> usize {\n    let r = $0(n).address() + 1$0;\n    r\n}\n", Some("usize")),
        ("use std::string::String as u32;\npub fn f(a: u32, b: u32) -> bool {\n    let r = $0a == b$0;\n    drop(a);\n    r && b.is_empty()\n}\n", Some("bool")),
        // Review of 9d489afe: a glob that may bring a primitive's name, an ASCII use of a name declared beyond ASCII
        // (the Kelvin sign is `K`), a type named with a combining mark, a local `const` or `static` initializer.
        ("mod types {\n    pub type u32 = String;\n}\nuse types::*;\npub fn f(a: u32, b: u32) -> bool {\n    let r = $0a == b$0;\n    drop(a);\n    r && b.is_empty()\n}\n", Some("bool")),
        ("const K: i32 = 100;\npub fn f() -> i32 {\n    const \u{212a}: i32 = 3;\n    let r = $0K + 1$0;\n    r\n}\n", Some("i32")),
        ("pub fn f(\u{212a}: i32) -> i32 {\n    let r = $0K + 1$0;\n    r\n}\n", Some("i32")),
        ("pub fn f() -> u8 {\n    type cafe\u{301} = u8;\n    let n: cafe\u{301} = $01 + 2$0;\n    n\n}\n", Some("u8")),
        ("pub fn f() -> i32 {\n    const C: i32 = {\n        let x: i32 = $01 + 2$0;\n        x\n    };\n    C\n}\n", Some("i32")),
        ("pub fn f() -> i32 {\n    static S: i32 = {\n        let x: i32 = $01 + 2$0;\n        x\n    };\n    S\n}\n", Some("i32")),
        // Review of 1db80426: a comparison borrows its operands, which a user's impl may observe.
        ("pub struct Ptr(pub *const u8);\nimpl PartialEq<&Ptr> for u8 {\n    fn eq(&self, p: &&Ptr) -> bool { std::ptr::eq(self, p.0) }\n}\npub fn f(n: u8, p: &Ptr) -> bool {\n    let r = $0n == p$0;\n    r\n}\n", Some("bool")),
        // Not a whole expression.
        ("pub fn f(a: u32, b: u32) -> u32 {\n    let c = $0a * b + $01;\n    c\n}\n", Some("u32")),
    ] {
        assert_eq!(extracted(doc, ty), None, "{doc}");
    }
}

/// Review of 531974ed: a range given end first is no selection, and no panic.
#[test]
fn test_a_reversed_range_is_declined() {
    let doc = "pub fn f(a: u32) -> u32 {\n    a + 1\n}\n";
    let range = json!({ "start": { "line": 1, "character": 9 }, "end": { "line": 1, "character": 4 } });
    assert_eq!(extract_fn_action(doc, "file:///w/src/lib.rs", &range, None, &|_| None), None);
}

/// Review of 7dca0b11: a macro called where an item or a statement goes may declare an import or an item the tree does
/// not show; the standard library's statement macros declare none.
#[test]
fn test_a_macro_that_may_declare_keeps_names_unknown() {
    for (doc, ty) in [
        ("mod types {\n    pub type u32 = String;\n}\nmacro_rules! import {\n    () => {\n        use types::*;\n    };\n}\nimport!();\npub fn f(a: u32, b: u32) -> bool {\n    let r = $0a == b$0;\n    r\n}\n", Some("bool")),
        ("const N: i32 = 100;\nmacro_rules! local {\n    () => {\n        const N: i32 = 3;\n    };\n}\npub fn f() -> i32 {\n    local!();\n    let r = $0N + 1$0;\n    r\n}\n", Some("i32")),
    ] {
        assert_eq!(extracted(doc, ty), None, "{doc}");
    }
    let doc = "pub fn f(a: u32) -> u32 {\n    println!(\"{a}\");\n    let r = $0a + 1$0;\n    r\n}\n";
    assert!(extracted(doc, Some("u32")).is_some(), "println! declares nothing");
}
