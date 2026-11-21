/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `Wrap return type in Option` / `in Result`: where offered, what it writes, and that the result compiles.

use super::*;
use crate::lsp::editing::tests::code_action_assists_tests::assert_rustc_compiles;

/// `doc` with the wrap at the first `$0` applied, nothing calling it from another file.
fn wrapped(doc: &str, wrap: Wrap) -> Option<String> {
    let offset = doc.find("$0").unwrap();
    let doc = doc.replacen("$0", "", 1);
    let mut out = doc.clone();
    for (r, text) in wrap_edits(&doc, offset, wrap, &|_| false)?.into_iter().rev() {
        out.replace_range(r, &text);
    }
    Some(out)
}

/// prod-code, m=20226: rust-analyzer offers both in `pub fn twice(x: u32) -> u32`.
#[test]
fn test_the_return_type_and_every_returned_value_are_wrapped() {
    let doc = "pub fn twice(x: u32) -> u$032 {\n    x * 2\n}\n";
    let got = wrapped(doc, Wrap::Option).unwrap();
    assert_eq!(got, "pub fn twice(x: u32) -> Option<u32> {\n    Some(x * 2)\n}\n");
    assert_rustc_compiles(&got, "Wrap return type in Option");
    let got = wrapped(doc, Wrap::Result).unwrap();
    assert_eq!(got, "pub fn twice(x: u32) -> Result<u32, ()> {\n    Ok(x * 2)\n}\n");
    assert_rustc_compiles(&got, "Wrap return type in Result");

    let doc = "pub fn $0pick(a: bool) -> &'static str {\n    if a {\n        return \"a\";\n    }\n    match a {\n        true => \"t\",\n        false => \"f\",\n    }\n}\n";
    let got = wrapped(doc, Wrap::Option).unwrap();
    assert!(got.contains("-> Option<&'static str>") && got.contains("return Some(\"a\");") && got.contains("Some(match a {"), "{got}");
    assert_rustc_compiles(&got, "Wrap return type in Option");

    let doc = "pub struct S;\nimpl S {\n    pub fn $0get(&self) -> u8 {\n        1\n    }\n}\n";
    assert_rustc_compiles(&wrapped(doc, Wrap::Result).unwrap(), "Wrap return type in Result");
}

#[test]
fn test_it_is_not_offered_where_the_result_would_not_compile() {
    for doc in [
        // A caller.
        "pub fn $0twice(x: u32) -> u32 {\n    x * 2\n}\npub fn four() -> u32 {\n    twice(2)\n}\n",
        // A signature a trait fixes.
        "pub trait T {\n    fn $0f(&self) -> u32;\n}\npub struct S;\nimpl T for S {\n    fn f(&self) -> u32 {\n        1\n    }\n}\n",
        "pub trait T {\n    fn f(&self) -> u32;\n}\npub struct S;\nimpl T for S {\n    fn $0f(&self) -> u32 {\n        1\n    }\n}\n",
        // Already wrapped, no return type, never returns, inside the body.
        "pub fn $0f() -> Option<u32> {\n    None\n}\n",
        "pub fn $0f() {\n}\n",
        "pub fn $0f() -> ! {\n    panic!()\n}\n",
        "pub fn f() -> u32 {\n    $01\n}\n",
    ] {
        assert_eq!(wrapped(doc, Wrap::Option), None, "{doc}");
    }
}
