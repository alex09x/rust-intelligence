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

/// prod-code, m=20226: rust-analyzer offers `inline_local_variable` at `let area = size * scale;`.
#[test]
fn test_a_binding_used_once_is_inlined() {
    let got = inlined("pub fn scaled(size: u32, scale: u32) -> u32 {\n    let $0area = size * scale;\n    area + 1\n}\n").unwrap();
    assert_eq!(got, "pub fn scaled(size: u32, scale: u32) -> u32 {\n    (size * scale) + 1\n}\n");
    assert_rustc_compiles(&got, "Inline variable");
    let got = inlined("pub fn g(a: u32) -> u32 {\n    let b$0 = a + 1u32;\n    b.pow(2)\n}\n").unwrap();
    assert_eq!(got, "pub fn g(a: u32) -> u32 {\n    (a + 1u32).pow(2)\n}\n");
    assert_rustc_compiles(&got, "Inline variable");
    // Nothing to reorder: it may cross a statement that does not name what it reads.
    let got = inlined("pub struct P { pub x: u8 }\npub fn h(p: &P) -> bool {\n    let f$0 = p.x;\n    let y = 3u8;\n    f == y\n}\n").unwrap();
    assert!(got.contains("    p.x == y\n") && !got.contains("let f"), "{got}");
    assert_rustc_compiles(&got, "Inline variable");
    // Review of 63f24322: a field called stays a field.
    let got = inlined("pub struct S { pub cb: fn() -> u8 }\npub fn k(s: &S) -> u8 {\n    let cb$0 = s.cb;\n    cb()\n}\n").unwrap();
    assert!(got.contains("    (s.cb)()\n"), "{got}");
    assert_rustc_compiles(&got, "Inline variable");
}

/// What rust-analyzer inlines though the result breaks or changes the code (review of 63f24322 included).
#[test]
fn test_a_binding_is_not_inlined_where_the_result_would_differ() {
    for doc in [
        // Moved twice.
        "pub fn f(s: String) -> (String, String) {\n    let t$0 = s;\n    (t.clone(), t)\n}\n",
        // Evaluated on every turn of the loop.
        "pub fn f(v: &mut Vec<u8>, n: u8) {\n    let m$0 = n + 1u8;\n    for _ in 0..3 {\n        v.push(m);\n    }\n}\n",
        // Captured by a closure.
        "pub fn f(v: u8) -> impl Fn() -> u8 {\n    let n$0 = v + 1u8;\n    move || n\n}\n",
        // What it reads changes before the use.
        "pub fn f(mut v: u8) -> u8 {\n    let n$0 = v;\n    v = 2u8;\n    n + v\n}\n",
        // A shadowed name in between, raw or not.
        "pub fn f(x: u32) -> u32 {\n    let a$0 = x;\n    let x = 5u32;\n    a + x\n}\n",
        "pub fn f(n: u8) -> u8 {\n    let x$0 = n;\n    let r#n = 2u8;\n    x + r#n\n}\n",
        // Effects run in another order, or not at all.
        "fn first() -> u8 { 1 }\nfn second() {}\npub fn f() -> u8 {\n    let x$0 = first();\n    second();\n    x\n}\n",
        "pub fn f(a: u8, c: bool) -> u8 {\n    let x$0 = a + 1u8;\n    if c { x } else { 0 }\n}\n",
        "fn g() -> u8 { 1 }\npub fn f(a: u8) -> u8 {\n    let x$0 = a * 2u8;\n    g() + x\n}\n",
        // A temporary borrowed, a literal typed again where it goes, a struct literal.
        "fn id(s: &String) -> &String { s }\npub fn f() -> usize {\n    let x$0 = &String::from(\"a\");\n    let y = id(x);\n    y.len()\n}\n",
        "pub fn f() -> u8 {\n    let n$0 = 256;\n    n as u8\n}\n",
        "pub struct S { pub n: u8 }\npub fn f() -> bool {\n    let s$0 = S { n: 1 };\n    if s.n > 0 { true } else { false }\n}\n",
        // Mutable, typed, a field shorthand, unused.
        "pub fn f() -> u32 {\n    let mut a$0 = 1u32;\n    a += 1;\n    a\n}\n",
        "pub fn f() -> u64 {\n    let a$0: u64 = 1;\n    a\n}\n",
        "pub struct S { pub a: u32 }\npub fn f() -> S {\n    let a$0 = 1u32;\n    S { a }\n}\n",
        "pub fn f() {\n    let a$0 = 1u8;\n}\n",
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
