/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The focused parse (`parse_for_offset`) keeps the item structure around the cursor the full parse sees: a `{ … }`
//! const argument in a header is no body.

use ridx_syntax::{parse, parse_for_offset, SyntaxKind as K, SyntaxTree};

/// The kinds from the `let` statement holding `offset` up to the root.
fn let_chain(tree: &SyntaxTree<'_>, offset: usize) -> Vec<K> {
    let at = tree
        .descendants(tree.root())
        .filter(|&n| tree.kind(n) == K::LetStmt && tree.range(n).contains(&offset))
        .last()
        .unwrap_or_else(|| panic!("no let at {offset} in {}", tree.dump(tree.root(), false)));
    tree.ancestors(at).map(|n| tree.kind(n)).collect()
}

fn assert_focused_matches_full(src: &str, needle: &str) {
    let off = src.find(needle).unwrap() + 4;
    let full = let_chain(&parse(src), off);
    let (focused, _) = parse_for_offset(src, off as u32);
    assert_eq!(let_chain(&focused, off), full, "focused parse of {src}");
    assert!(full.contains(&K::Impl), "the method stays in its impl: {full:?}");
}

const FILLER: &str = "fn a() { let q = 1; }\nfn b() -> u8 { 2 }\n";

#[test]
fn test_const_argument_block_in_an_impl_header_is_no_body() {
    let src = format!("{FILLER}impl<T> Matrix<T, {{ 1 + 1 }}> {{\n    fn f(&self) {{ let x = self; }}\n}}\n{FILLER}");
    assert_focused_matches_full(&src, "let x");
}

#[test]
fn test_const_argument_block_in_a_skipped_return_type() {
    let src = format!("{FILLER}fn helper() -> Array<u8, {{ 1 }}> {{ todo!() }}\nimpl S {{\n    fn f(&self) {{ let x = self; }}\n}}\n");
    assert_focused_matches_full(&src, "let x");
}

#[test]
fn test_arrows_and_nested_generics_in_headers_still_close() {
    // `->` is no closing angle; `>>` closes two: the body after each header is found and skipped as before.
    let src = format!(
        "fn mk() -> Box<dyn Fn(u8) -> u8> {{ Box::new(|v| v) }}\nfn w<T: Into<Vec<u8>>>(t: T) -> Vec<u8> where T: Clone {{ t.into() }}\nimpl<F: Fn() -> u8> Run for Wrap<F> {{ fn r(&self) {{ let z = 1; }} }}\n{FILLER}impl S {{\n    fn f(&self) {{ let x = self; }}\n}}\n"
    );
    assert_focused_matches_full(&src, "let x");
    let off = src.find("let x").unwrap() as u32 + 4;
    let (focused, span) = parse_for_offset(&src, off);
    let fns = focused.descendants(focused.root()).filter(|&n| focused.kind(n) == K::Fn).count();
    assert!(fns >= 4, "the other items are kept as items: {}", focused.dump(focused.root(), false));
    let (s, e) = span.unwrap();
    assert!(s <= off && off <= e && (e - s) < 40, "the focus is the method itself: {span:?}");
}

#[test]
fn test_const_argument_block_in_a_skipped_impl_before_the_target() {
    let src = format!("impl<T> Matrix<T, {{ 2 }}> {{ fn g(&self) {{ let y = 1; }} }}\n{FILLER}impl S {{\n    fn f(&self) {{ let x = self; }}\n}}\n");
    assert_focused_matches_full(&src, "let x");
}
