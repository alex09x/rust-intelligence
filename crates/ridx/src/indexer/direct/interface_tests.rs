/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::*;

fn same(a: &str, b: &str) -> bool {
    file_interface(a).unwrap().hash == file_interface(b).unwrap().hash
}

const BASE: &str = "/// Adds.\npub fn add(a: u32, b: u32) -> u32 {\n    a + b\n}\n\npub struct Pair {\n    pub left: u32,\n}\n";

#[test]
fn test_a_body_or_a_comment_is_no_change_of_the_interface() {
    assert!(same(BASE, &BASE.replace("a + b", "let s = a + b;\n    // sum\n    s")), "a body, lines added to it");
    assert!(same(BASE, &BASE.replace("a + b", "/// in a body\n    a + b")), "a doc comment in a body left out");
    assert!(same(BASE, &format!("// header\n\n{BASE}// trailer\n//// not a doc\n/* block */\n")), "comments and blank lines");
    assert!(same("pub fn f() { #[allow(unused)] let x = 1; }", "pub fn f() { #[allow(unused)] let x = 2; }"), "a built-in attribute in a body");
    // Review of 6b27649c: `line!()` kept moves with the text; in a body left out it is no interface.
    assert!(!same("pub const N: u32 = line!();", "\npub const N: u32 = line!();"), "a line above `line!()`");
    assert!(same("pub fn f() -> u32 { line!() }", "\npub fn f() -> u32 { line!() }"));
    assert!(same(BASE, &BASE.replace("a + b", "println!(\"{a}\");\n    assert!(a > 0);\n    a + b")), "the standard library's macros");
    assert!(same("trait T { fn f(&self) -> u8 { 1 } }", "trait T { fn f(&self) -> u8 { 2 } }"), "a trait's default body");
}

#[test]
fn test_what_a_dependent_sees_changes_the_interface() {
    assert!(!same(BASE, &BASE.replace("b: u32) -> u32", "b: u64) -> u32")), "a signature");
    assert!(!same(BASE, &BASE.replace("pub left: u32", "pub left: u32,\n    right: u8")), "a private field changes the layout");
    assert!(!same(BASE, &BASE.replace("pub fn add", "fn add")), "a visibility");
    assert!(!same(BASE, &format!("{BASE}pub fn more() {{}}\n")), "an item added");
    assert!(!same("pub const N: usize = 3;", "pub const N: usize = 4;"), "a constant's value");
    assert!(!same("#[doc(hidden)] pub fn f() {}", "pub fn f() {}"), "an attribute");
    // Review of 2a030462: what a macro is given.
    assert!(!same(BASE, &BASE.replace("/// Adds.", "/// Adds two numbers.")), "a doc comment is an attribute a derive or a macro reads");
    assert!(!same("make! { /// One.\n pub struct S; }", "make! { /// Two.\n pub struct S; }"), "a doc comment in a macro call");
    assert!(!same("make!(>>);", "make!(> >);"), "joint punctuation");
}

#[test]
fn test_bodies_a_dependent_can_see_into_stay() {
    for (a, b) in [
        ("pub const fn n() -> usize { 3 }", "pub const fn n() -> usize { 4 }"),
        ("pub const unsafe fn n() -> usize { 3 }", "pub const unsafe fn n() -> usize { 4 }"),
        ("pub async fn n() -> u8 { 3 }", "pub async fn n() -> u8 { 4 }"),
        ("pub fn n() -> impl Fn() -> u8 { || 3 }", "pub fn n() -> impl Fn() -> u8 { || 4 }"),
        ("pub fn n() -> (impl Clone, u8) { (1, 3) }", "pub fn n() -> (impl Clone, u8) { (1, 4) }"),
        ("pub fn n() { impl super::T for u8 {} }", "pub fn n() { impl super::T for u16 {} }"),
        ("pub fn n() { macro_rules! m { () => {} } }", "pub fn n() { macro_rules! m { (x) => {} } }"),
        ("#[tracing::instrument] pub fn n() -> u8 { 3 }", "#[tracing::instrument] pub fn n() -> u8 { 4 }"),
        ("#[my_attr] impl S { pub fn n(&self) -> u8 { 3 } }", "#[my_attr] impl S { pub fn n(&self) -> u8 { 4 } }"),
        // Review of 2a030462: not cut at the `,` of its generics.
        ("#[my_attr] impl<T, U> S<T, U> { pub fn n(&self) -> u8 { 3 } }", "#[my_attr] impl<T, U> S<T, U> { pub fn n(&self) -> u8 { 4 } }"),
        ("pub fn f() { add!(u8); }", "pub fn f() { add!(i8); }"),
        // Review of 6b27649c: an attribute macro inside a body.
        ("pub fn f() { #[make_impl(u8)] fn local() {} }", "pub fn f() { #[make_impl(u16)] fn local() {} }"),
        ("pub fn f() { other::add!(u8); }", "pub fn f() { other::add!(i8); }"),
        ("macro_rules! m { () => { fn f() { 1 } } }", "macro_rules! m { () => { fn f() { 2 } } }"),
        ("m! { fn f() { 1 } }", "m! { fn f() { 2 } }"),
        ("pub fn n<const N: usize>() -> [u8; { N }] where [u8; { N }]: Sized { [0; N] }", "pub fn n<const N: usize>() -> [u16; { N }] where [u8; { N }]: Sized { [0; N] }"),
    ] {
        assert!(!same(a, b), "{a}");
    }
    // A plain attribute leaves the body out; an argument-position `impl` is no hidden type.
    assert!(same("#[inline]\n#[must_use]\npub fn n(x: impl Into<u8>) -> u8 { x.into() }", "#[inline]\n#[must_use]\npub fn n(x: impl Into<u8>) -> u8 { 3 }"));
    assert!(same("#[unsafe(no_mangle)] pub extern \"C\" fn n() -> u8 { 3 }", "#[unsafe(no_mangle)] pub extern \"C\" fn n() -> u8 { 4 }"));
}

#[test]
fn test_items_under_cfg_test_are_left_out() {
    let lib = "pub fn api() {}\n";
    assert!(same(lib, &format!("{lib}#[cfg(test)]\nmod tests {{\n    #[test]\n    fn t() {{ super::api(); }}\n}}\n")));
    assert!(same("pub struct S {\n    #[cfg(test)]\n    probe: u8,\n    pub a: u8,\n}", "pub struct S {\n    pub a: u8,\n}"), "a field");
    assert!(!same(lib, &format!("{lib}#[cfg(not(test))]\npub fn other() {{}}\n")), "any other cfg stays");
}

#[test]
fn test_positions_follow_the_tokens_kept() {
    let before = file_interface(BASE).unwrap();
    let after = file_interface(&BASE.replace("a + b", "let s = a + b;\n    s").replace("/// Adds.", "// é\n/// Adds.")).unwrap();
    assert_eq!(before.hash, after.hash);
    assert_eq!(before.positions.len(), after.positions.len());
    let at = |f: &FileInterface, line: u32, col: u32| f.positions.iter().position(|&p| p == (line, col));
    // `add` at 2:8, `Pair` at 6:12 before; one line more above `add`, two more above `Pair`.
    let add = at(&before, 2, 8).expect("`add` is kept");
    assert_eq!(after.positions[add], (3, 8));
    let pair = at(&before, 6, 12).expect("`Pair` is kept");
    assert_eq!(after.positions[pair], (8, 12));
    // Columns count characters, as the driver does.
    let wide = file_interface("const É: u8 = 0; pub fn f() {}").unwrap();
    assert!(wide.positions.contains(&(1, 25)), "`f` after a two-byte character: {:?}", wide.positions);
}

#[test]
fn test_unbalanced_text_has_no_interface() {
    assert!(file_interface("pub fn f() {").is_none());
    assert!(file_interface("pub fn f() }").is_none());
    assert!(file_interface("pub fn f() {}").is_some());
}

#[test]
fn test_fnv_matches_the_format_hash() {
    let mut h = Fnv::new();
    h.write(b"ab");
    h.write(b"c");
    assert_eq!(h.finish(), ridx_format::fnv1a_64(b"abc"));
}
