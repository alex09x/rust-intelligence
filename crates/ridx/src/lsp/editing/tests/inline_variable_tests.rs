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
    let got = inlined("pub fn f(v: &[u8]) -> usize {\n    let n$0 = v.len();\n    std::cmp::max(n, 1)\n}\n").unwrap();
    assert_eq!(got, "pub fn f(v: &[u8]) -> usize {\n    std::cmp::max(v.len(), 1)\n}\n", "no parentheses in an argument");
    assert_rustc_compiles(&got, "Inline variable");
    let got = inlined("pub fn g(a: u32) -> u32 {\n    let b$0 = a + 1;\n    b.pow(2)\n}\n").unwrap();
    assert_eq!(got, "pub fn g(a: u32) -> u32 {\n    (a + 1).pow(2)\n}\n");
    assert_rustc_compiles(&got, "Inline variable");
}

/// What rust-analyzer inlines though the result breaks or changes the code.
#[test]
fn test_a_binding_is_not_inlined_where_the_result_would_differ() {
    for doc in [
        // Moved twice.
        "pub fn f(s: String) -> (String, String) {\n    let t$0 = s;\n    (t.clone(), t)\n}\n",
        // Evaluated on every turn of the loop.
        "pub fn f(v: &mut Vec<u8>) {\n    let n$0 = v.len() as u8;\n    for _ in 0..3 {\n        v.push(n);\n    }\n}\n",
        // Captured by a closure.
        "pub fn f(v: Vec<u8>) -> impl Fn() -> usize {\n    let n$0 = v.len();\n    move || n\n}\n",
        // What it reads changes before the use.
        "pub fn f(mut v: Vec<u8>) -> usize {\n    let n$0 = v.len();\n    v.push(1);\n    n\n}\n",
        // A shadowed name in between.
        "pub fn f(x: u32) -> u32 {\n    let a$0 = x;\n    let x = 5;\n    a + x\n}\n",
        // Mutable, typed, a field shorthand, unused.
        "pub fn f() -> u32 {\n    let mut a$0 = 1;\n    a += 1;\n    a\n}\n",
        "pub fn f() -> u64 {\n    let a$0: u64 = 1;\n    a\n}\n",
        "pub struct S { pub a: u32 }\npub fn f() -> S {\n    let a$0 = 1;\n    S { a }\n}\n",
        "pub fn f() {\n    let a$0 = 1;\n}\n",
    ] {
        assert_eq!(inlined(doc), None, "{doc}");
    }
}

#[test]
fn test_the_action_carries_its_edit() {
    let doc = "pub fn f() -> u32 {\n    let a = 2;\n    a\n}\n";
    let range = json!({ "start": { "line": 1, "character": 8 }, "end": { "line": 1, "character": 8 } });
    let action = inline_variable_action(doc, "file:///w/src/lib.rs", &range, Some(3)).unwrap();
    assert_eq!(action["title"], "Inline variable");
    let edits = &action["edit"]["documentChanges"][0]["edits"];
    assert_eq!(edits[0]["range"], json!({ "start": { "line": 1, "character": 0 }, "end": { "line": 2, "character": 0 } }));
    assert_eq!(edits[1], json!({ "range": { "start": { "line": 2, "character": 4 }, "end": { "line": 2, "character": 5 } }, "newText": "2" }));
}
