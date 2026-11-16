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

/// The 1-based `(line, col)` of `nth` (0-based) occurrence of the word `w` in `code`.
fn at(code: &str, w: &str, nth: usize) -> (u32, u32) {
    let p = code
        .match_indices(w)
        .filter(|(p, _)| !code[..*p].ends_with(|c: char| c.is_alphanumeric() || c == '_') && !code[p + w.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_'))
        .nth(nth)
        .unwrap()
        .0;
    let line = code[..p].matches('\n').count() as u32 + 1;
    let col = (p - code[..p].rfind('\n').map_or(0, |i| i + 1)) as u32 + 1;
    (line, col)
}

fn linked_word(code: &str, w: &str, nth: usize) -> Vec<(u32, u32)> {
    let (l, c) = at(code, w, nth);
    linked(code, l, c)
}

fn occurrences(code: &str, w: &str, which: &[usize]) -> Vec<(u32, u32)> {
    let mut v: Vec<(u32, u32)> = which.iter().map(|&n| at(code, w, n)).collect();
    v.sort();
    v
}

#[test]
fn test_closures_after_comments_and_references_and_no_or_pattern_pipe() {
    let code = "fn f(v: u8) -> u8 {\n    let g = /* c */ |v: u8| v + 1;\n    let h = &|v: u8| v;\n    let k = async |v: u8| v;\n    let r = match Some(v) {\n        | Some(v) => { let a = 1; v + a }\n        | None => v,\n    };\n    g(v) + h(v) + r\n}\n";
    // Occurrences of `v`: 0 param, 1-2 g, 3-4 h, 5-6 k, 7 scrutinee, 8-9 arm, 10 None arm, 11-12 calls.
    assert_eq!(linked_word(code, "v", 0), occurrences(code, "v", &[0, 7, 10, 11, 12]), "the parameter: closures and the arm shadow it");
    assert_eq!(linked_word(code, "v", 1), occurrences(code, "v", &[1, 2]), "after a block comment");
    assert_eq!(linked_word(code, "v", 3), occurrences(code, "v", &[3, 4]), "after `&`");
    assert_eq!(linked_word(code, "v", 5), occurrences(code, "v", &[5, 6]), "after `async`");
    assert_eq!(linked_word(code, "v", 8), occurrences(code, "v", &[8, 9]), "a leading `|` of an arm is no closure");
}

#[test]
fn test_header_blocks_block_arms_or_patterns_let_chains_and_raw_names() {
    let header = "fn f(x: u8, xs: &[u8]) {\n    for x in unsafe { xs } {\n        let _ = x;\n    }\n    if let Some(x) = { Some(x) } {\n        let _ = x;\n    }\n}\n";
    assert_eq!(linked_word(header, "x", 1), occurrences(header, "x", &[1, 2]), "the loop body after an `unsafe` block");
    assert_eq!(linked_word(header, "x", 3), occurrences(header, "x", &[3, 5]), "the body after a block scrutinee");
    let arms = "fn f(v: Option<u8>, c: bool) -> u8 {\n    match v {\n        Some(v) => if c { v } else { 0 }\n        None => v.map_or(0, |w| w),\n    }\n}\n";
    // Occurrences of `v`: 0 param, 1 scrutinee, 2 `Some(v)`, 3 its use, 4 the `None` arm's.
    assert_eq!(linked_word(arms, "v", 2), occurrences(arms, "v", &[2, 3]), "a comma-less `if` arm ends at its closing brace");
    assert_eq!(linked_word(arms, "v", 0), occurrences(arms, "v", &[0, 1, 4]), "`v` of the next arm is the parameter");
    let or = "fn f(r: Result<u8, u8>) -> u8 {\n    match r {\n        Ok(v) | Err(v) => v,\n    }\n}\n";
    let all = occurrences(or, "v", &[0, 1, 2]);
    assert_eq!(linked_word(or, "v", 0), all, "both branches bind one `v`");
    assert_eq!(linked_word(or, "v", 1), all);
    let chain = "fn f(v: Option<u8>) -> u8 {\n    if let Some(v) = v && v > 0 {\n        return v;\n    }\n    0\n}\n";
    assert_eq!(linked_word(chain, "v", 1), occurrences(chain, "v", &[1, 3, 4]), "seen after `&&` of the let chain");
    let raw = "fn f(pair: (u8, u8)) -> u8 {\n    let (r#type, v) = pair;\n    r#type + v\n}\n";
    assert_eq!(linked_word(raw, "r#type", 0), occurrences(raw, "r#type", &[0, 1]));
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

#[test]
fn test_header_blocks_with_else_chains_and_operators() {
    let code = "fn f(x: Vec<u8>, c: bool, v: Option<u8>, other: u8) -> u8 {\n    for x in if c { x.clone() } else { vec![] } {\n        let _ = x;\n    }\n    if let Some(v) = { v } && v > 0 {\n        return v;\n    }\n    match v {\n        Some(v) => { v } + other,\n        None => other,\n    }\n}\n";
    // `x`: 0 param, 1 loop binding, 2 `x.clone()` (the param), 3 the body's.
    assert_eq!(linked_word(code, "x", 1), occurrences(code, "x", &[1, 3]), "the loop body after `if .. else ..`");
    // `v`: 0 param, 1 `Some(v)` of the if let, 2 `{ v }` (the param), 3 after `&&`, 4 `return v`.
    assert_eq!(linked_word(code, "v", 1), occurrences(code, "v", &[1, 3, 4]), "the body after a block scrutinee and a let chain");
    // `other`: 0 param, 1 the arm `{ v } + other`, 2 the `None` arm's: no false binding from `+ other`.
    assert_eq!(linked_word(code, "other", 0), occurrences(code, "other", &[0, 1, 2]));
}
