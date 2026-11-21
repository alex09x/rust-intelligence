/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `Inline variable`: where it is offered, what it writes, and that the result compiles.

use super::*;
use crate::lsp::editing::tests::code_action_assists_tests::assert_rustc_compiles;

/// `doc` with the action at the first `$0` applied, or `None` where it is not offered.
fn inlined(doc: &str) -> Option<String> {
    let offset = doc.find("$0").unwrap();
    let doc = doc.replacen("$0", "", 1);
    let mut out = doc.clone();
    for (r, text) in inline_edits(&doc, offset)?.into_iter().rev() {
        out.replace_range(r, &text);
    }
    Some(out)
}

/// prod-code, m=20226: rust-analyzer offers `inline_local_variable` at a `let` used once; ridx where the initializer
/// is a value with nothing to evaluate.
#[test]
fn test_a_binding_used_once_is_inlined() {
    let got = inlined("pub fn scaled(size: u32, scale: u32) -> u32 {\n    let $0n = 4u32;\n    size * scale + n\n}\n").unwrap();
    assert_eq!(got, "pub fn scaled(size: u32, scale: u32) -> u32 {\n    size * scale + 4u32\n}\n");
    assert_rustc_compiles(&got, "Inline variable");
    // A cast, parenthesized where it goes.
    let got = inlined("pub fn g(a: u8) -> u64 {\n    let b$0 = a as u64;\n    b * 2\n}\n").unwrap();
    assert_eq!(got, "pub fn g(a: u8) -> u64 {\n    (a as u64) * 2\n}\n");
    assert_rustc_compiles(&got, "Inline variable");
    // Across a statement that does not name what it reads.
    let got = inlined("pub fn h(p: u8) -> u8 {\n    let f$0 = p;\n    let y = 3u8;\n    f + y\n}\n").unwrap();
    assert!(got.contains("    p + y\n") && !got.contains("let f"), "{got}");
    assert_rustc_compiles(&got, "Inline variable");
    // A local with its type written, a tuple, a shared reference.
    let got = inlined("fn take(t: (u8, &str)) -> u8 { t.0 }\npub fn k(s: &str) -> u8 {\n    let n: u8 = 2;\n    let t$0 = (n, s);\n    take(t)\n}\n").unwrap();
    assert!(got.contains("    take((n, s))\n"), "{got}");
    assert_rustc_compiles(&got, "Inline variable");
}

/// What rust-analyzer inlines though the result breaks or changes the code (reviews of 63f24322 and 36349abf).
#[test]
fn test_a_binding_is_not_inlined_where_the_result_would_differ() {
    for doc in [
        // Moved twice, or moved at all (a value with a `Drop` moved later).
        "pub fn f(s: String) -> (String, String) {\n    let t$0 = s;\n    (t.clone(), t)\n}\n",
        "pub fn f(s: String, g: String) -> (String, String) {\n    let x$0 = s;\n    let y = g;\n    (y, x)\n}\n",
        // Evaluated on every turn of the loop, captured by a closure, deferred by an `async` block.
        "pub fn f(v: &mut Vec<u8>, n: u8) {\n    let m$0 = n + 1u8;\n    for _ in 0..3 {\n        v.push(m);\n    }\n}\n",
        "pub fn f(v: u8) -> impl Fn() -> u8 {\n    let n$0 = v + 1u8;\n    move || n\n}\n",
        "pub fn f(a: u8) -> impl std::future::Future<Output = u8> {\n    let x$0 = a + 1u8;\n    async move { x }\n}\n",
        // What it reads changes before the use.
        "pub fn f(mut v: u8) -> u8 {\n    let n$0 = v;\n    v = 2u8;\n    n + v\n}\n",
        // A shadowed name in between, raw or not.
        "pub fn f(x: u32) -> u32 {\n    let a$0 = x;\n    let x = 5u32;\n    a + x\n}\n",
        "pub fn f(n: u8) -> u8 {\n    let x$0 = n;\n    let r#n = 2u8;\n    x + r#n\n}\n",
        // Effects run in another order, or not at all: a call, an assignment, an overloaded operator.
        "fn first() -> u8 { 1 }\nfn second() {}\npub fn f() -> u8 {\n    let x$0 = first();\n    second();\n    x\n}\n",
        "pub fn f(a: u8, c: bool) -> u8 {\n    let x$0 = a + 1u8;\n    if c { x } else { 0 }\n}\n",
        "pub fn f(mut a: u8) -> u8 {\n    let x$0 = a = 1u8;\n    let _ = x;\n    a\n}\n",
        "#[derive(PartialEq)]\npub struct S;\npub fn f(a: &S, b: &S) -> bool {\n    let e$0 = a == b;\n    e\n}\n",
        // Another panic or exit first: an arithmetic or a `?` evaluated before the use.
        "pub fn f(a: u8, c: u8, d: u8) -> u8 {\n    let x$0 = a + 1u8;\n    c / d + x\n}\n",
        "pub fn f(a: u8, r: Result<u8, ()>) -> Result<u8, ()> {\n    let x$0 = a + 1u8;\n    Ok(r? + x)\n}\n",
        // The binding's own storage seen: borrowed, a receiver.
        "pub fn f(n: u8) -> bool {\n    let x$0 = n;\n    std::ptr::eq(&x, &n)\n}\n",
        "pub fn f(a: u32) -> u32 {\n    let b$0 = a + 1u32;\n    b.pow(2)\n}\n",
        // A temporary borrowed, a literal typed again where it goes (hexadecimal digits are no suffix), a field.
        "const EMPTY: String = String::new();\npub fn f() -> usize {\n    let x$0 = &EMPTY;\n    x.len()\n}\n",
        "pub fn f() -> u8 {\n    let n$0 = 256;\n    n as u8\n}\n",
        "pub fn f() -> u8 {\n    let n$0 = 0xff32;\n    n as u8\n}\n",
        "pub struct S { pub cb: fn() -> u8 }\npub fn f(s: &S) -> u8 {\n    let cb$0 = s.cb;\n    cb()\n}\n",
        // Mutable, typed, a field shorthand, unused.
        "pub fn f() -> u32 {\n    let mut a$0 = 1u32;\n    a += 1;\n    a\n}\n",
        "pub fn f() -> u64 {\n    let a$0: u64 = 1;\n    a\n}\n",
        "pub struct S { pub a: u32 }\npub fn f() -> S {\n    let a$0 = 1u32;\n    S { a }\n}\n",
        "pub fn f() {\n    let a$0 = 1u8;\n}\n",
        // Review of 03933e37: any operator (its overflow, or what is evaluated before it, moves: prod-code's own
        // `size * scale` too), a type only inferred (the index may be older than the code), a borrow or a receiver in
        // parentheses, a name a later item or another binding shares, a name beyond ASCII where it goes.
        "pub fn scaled(size: u32, scale: u32) -> u32 {\n    let $0area = size * scale;\n    area + 1\n}\n",
        "pub fn f(v: &mut [u8], a: u8) {\n    let x$0 = a + 1u8;\n    v[9] = x;\n}\n",
        "fn g() -> u8 { 1 }\npub fn f() -> u8 {\n    let n = g();\n    let x$0 = n;\n    x\n}\n",
        "pub fn f(n: u8) -> bool {\n    let x$0 = n;\n    std::ptr::eq(&(x), &n)\n}\n",
        "pub fn f(a: u32) -> u32 {\n    let b$0 = a;\n    (b).pow(2)\n}\n",
        "pub fn f(n: u8) -> u8 {\n    let x$0 = n;\n    {\n        const n: u8 = 9;\n        x + n\n    }\n}\n",
        "pub fn f(n: u8, c: bool) -> u8 {\n    let x$0 = n;\n    if c {\n        let n = 2u8;\n        return n;\n    }\n    x\n}\n",
        "pub fn f(n: u8) -> u8 {\n    let x$0 = n;\n    let caf\u{e9} = 1u8;\n    x + caf\u{e9}\n}\n",
        // Review of b854363f: a `ref` pattern and a comparison borrow the binding's storage; a later `use` in a block
        // may name what the initializer reads; a glob may make `u8` a type with a destructor.
        "pub fn f(a: u8) -> bool {\n    let x$0 = a;\n    match x {\n        ref r => std::ptr::eq(r, &a),\n    }\n}\n",
        "pub struct P(pub *const u8);\nimpl PartialEq<P> for u8 {\n    fn eq(&self, p: &P) -> bool { std::ptr::eq(self, p.0) }\n}\npub fn f(a: u8, p: P) -> bool {\n    let x$0 = a;\n    x == p\n}\n",
        "mod values {\n    pub const n: u8 = 2;\n}\npub fn f(n: u8) -> u8 {\n    let x$0 = n;\n    {\n        use crate::values::n;\n        x + 0\n    }\n}\n",
        "mod t {\n    pub struct u8;\n}\nuse t::*;\npub fn f(s: u8) -> (u8, u8) {\n    let x$0 = s;\n    (x, u8)\n}\n",
    ] {
        assert_eq!(inlined(doc), None, "{doc}");
    }
}

#[test]
fn test_the_action_carries_its_edit() {
    let doc = "pub fn f() -> u32 {\n    let a = 2u32;\n    a\n}\n";
    let range = json!({ "start": { "line": 1, "character": 8 }, "end": { "line": 1, "character": 8 } });
    let action = inline_variable_action(doc, "file:///w/src/lib.rs", &range, Some(3)).unwrap();
    assert_eq!(action["title"], "Inline variable");
    let edits = &action["edit"]["documentChanges"][0]["edits"];
    assert_eq!(edits[0]["range"], json!({ "start": { "line": 1, "character": 0 }, "end": { "line": 2, "character": 0 } }));
    assert_eq!(edits[1], json!({ "range": { "start": { "line": 2, "character": 4 }, "end": { "line": 2, "character": 5 } }, "newText": "2u32" }));
}
