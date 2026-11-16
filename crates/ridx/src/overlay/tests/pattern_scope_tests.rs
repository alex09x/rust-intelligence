/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Names bound by patterns shadow an outer binding of the same name: in Zed, typing into `let v` with linked editing
//! renamed the closure parameter of `|v: u64| v + 1` too, as did rename and highlight, the overlay knowing only `let`
//! and fn parameters.

use crate::overlay::DocumentOverlay;

/// The 1-based `(line, col)` of each occurrence that resolves to the binding at `line:col` of `code`.
fn linked(code: &str, line: u32, col: u32) -> Vec<(u32, u32)> {
    let ov = DocumentOverlay::new("file:///t.rs", "/t.rs", 1, code.to_string());
    let mut out: Vec<(u32, u32)> = ov.find_local_binding_references(line, col).into_iter().map(|(l, c, _, _)| (l, c)).collect();
    out.sort();
    out
}

#[test]
fn test_closure_parameter_shadows_the_outer_binding() {
    let code = "fn shadow(n: u64) -> u64 {\n    let v = n + 1;\n    let v = v * 2;\n    let f = |v: u64| v + 1;\n    f(v) + n\n}\n";
    assert_eq!(linked(code, 2, 9), vec![(2, 9), (3, 13)], "the first `v`");
    assert_eq!(linked(code, 3, 9), vec![(3, 9), (5, 7)], "the second `v`, not the closure's");
    assert_eq!(linked(code, 4, 14), vec![(4, 14), (4, 22)], "the closure parameter");
    let block = "fn f(x: u8) {\n    let g = move |x| {\n        let y = x;\n        y\n    };\n    g(x);\n}\n";
    assert_eq!(linked(block, 1, 6), vec![(1, 6), (6, 7)]);
    assert_eq!(linked(block, 2, 19), vec![(2, 19), (3, 17)]);
    assert_eq!(linked("fn f(a: bool, b: bool) -> bool {\n    a || b\n}\n", 2, 5), vec![(1, 6), (2, 5)], "`||` is no closure");
}

#[test]
fn test_for_if_let_match_and_destructuring_bind_their_names() {
    let code = "fn f(v: Option<u8>, xs: &[u8]) -> u8 {\n    for v in xs {\n        let _ = v;\n    }\n    if let Some(v) = v {\n        return v;\n    }\n    match v {\n        Some(v) if v > 1 => v,\n        _ => 0,\n    };\n    let (a, v) = (1, v);\n    a + v.unwrap_or(0)\n}\n";
    assert_eq!(linked(code, 1, 6), vec![(1, 6), (5, 22), (8, 11), (12, 22)], "the parameter: the scrutinees and initializers only");
    assert_eq!(linked(code, 2, 9), vec![(2, 9), (3, 17)], "the loop variable");
    assert_eq!(linked(code, 5, 17), vec![(5, 17), (6, 16)], "the `if let` binding");
    assert_eq!(linked(code, 9, 14), vec![(9, 14), (9, 20), (9, 29)], "the arm binding, in its guard and expression");
    assert_eq!(linked(code, 12, 13), vec![(12, 13), (13, 9)], "the destructured `v`");
    assert_eq!(linked(code, 12, 10), vec![(12, 10), (13, 5)]);
}
