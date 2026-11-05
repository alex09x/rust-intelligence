/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Declarations cut from source as rust-analyzer's hover prints them.

use super::*;
use crate::lsp::intelligence::hover_generics::{generic_impl_header, with_impl_bounds};

fn item_at(src: &str, line: u32) -> usize {
    skip_trivia(src, line_start(src, line).unwrap())
}

#[test]
fn test_fn_header_keeps_names_generics_and_lays_out_where() {
    let src = "/// Doc.\n#[inline]\npub fn render<T>(\n    value: &T, // the value\n    width: usize,\n) -> String\nwhere\n    T: std::fmt::Display + ?Sized,\n{\n    todo!()\n}\n";
    let at = item_at(src, 3);
    let (end, delim) = header_end(src, at);
    assert_eq!(delim, Some(b'{'));
    assert_eq!(with_where_layout(&src[at..end]), "pub fn render<T>(value: &T, width: usize) -> String\nwhere\n    T: std::fmt::Display + ?Sized,");
}

#[test]
fn test_trait_method_declaration_ends_at_semicolon() {
    let src = "    fn serialize_some<T>(self, value: &T) -> Result<Self::Ok, Self::Error>\n    where\n        T: ?Sized + Serialize;\n";
    let at = item_at(src, 1);
    let (end, delim) = header_end(src, at);
    assert_eq!(delim, Some(b';'));
    assert_eq!(with_where_layout(&src[at..end]), "fn serialize_some<T>(self, value: &T) -> Result<Self::Ok, Self::Error>\nwhere\n    T: ?Sized + Serialize,");
}

#[test]
fn test_struct_lists_fields_without_docs_and_attributes() {
    let src = "#[derive(Debug)]\npub struct Circle {\n    /// The radius.\n    pub radius: f64,\n    #[allow(dead_code)]\n    center: (f64, f64),\n}\n";
    assert_eq!(render_adt(src, item_at(src, 1), false).unwrap(), "pub struct Circle {\n    pub radius: f64,\n    center: (f64, f64),\n}");
}

#[test]
fn test_struct_with_where_clause_and_many_fields() {
    let src = "pub struct Big<T>\nwhere\n    T: Clone,\n{\n    a: u8,\n    b: u8,\n    c: u8,\n    d: u8,\n    e: u8,\n    f: Vec<T>,\n}\n";
    assert_eq!(render_adt(src, 0, false).unwrap(), "pub struct Big<T>\nwhere\n    T: Clone,\n{\n    a: u8,\n    b: u8,\n    c: u8,\n    d: u8,\n    e: u8,\n    /* … */\n}");
}

#[test]
fn test_enum_elides_variant_fields_and_variant_shows_them() {
    let src = "pub enum Kind {\n    /// Round.\n    Round,\n    Cornered { corners: u32 },\n    Pair(u8, u8),\n    Code = 3,\n}\n";
    assert_eq!(render_adt(src, 0, true).unwrap(), "pub enum Kind {\n    Round,\n    Cornered { /* … */ },\n    Pair(/* … */),\n    Code = 3,\n}");
    assert_eq!(render_variant(src, item_at(src, 4)).unwrap(), "Cornered { pub corners: u32, }");
    assert_eq!(render_variant(src, item_at(src, 5)).unwrap(), "Pair(u8, u8)");
    assert_eq!(render_variant(src, item_at(src, 3)).unwrap(), "Round");
    assert_eq!(render_variant(src, item_at(src, 6)).unwrap(), "Code = 3");
}

#[test]
fn test_field_and_tuple_and_unit_structs() {
    let src = "pub struct S {\n    pub radius: f64,\n    map: HashMap<u8, Vec<u8>>,\n}\npub struct T(pub u8);\npub struct U;\n";
    assert_eq!(render_field(src, item_at(src, 2)).unwrap(), "pub radius: f64");
    assert_eq!(render_field(src, item_at(src, 3)).unwrap(), "map: HashMap<u8, Vec<u8>>");
    assert_eq!(render_adt(src, item_at(src, 5), false).unwrap(), "pub struct T(pub u8)");
    assert_eq!(render_adt(src, item_at(src, 6), false).unwrap(), "pub struct U");
}

#[test]
fn test_generic_impl_header_only_for_generic_impls() {
    let src = "impl<T, A: Allocator> Vec<T, A>\nwhere\n    T: Clone,\n{\n    /// Push.\n    pub fn push(&mut self, value: T) {}\n}\nimpl Circle {\n    pub fn new() -> Self { Circle }\n}\npub trait Shape {\n    fn area(&self) -> f64;\n}\n";
    let (header, bounds) = generic_impl_header(src, item_at(src, 6)).unwrap();
    assert_eq!(header, "impl<T, A> Vec<T, A>");
    assert_eq!(bounds, vec!["A: Allocator".to_string(), "T: Clone".to_string()]);
    assert_eq!(
        with_impl_bounds("pub fn push(&mut self, value: T)".into(), &bounds),
        "pub fn push(&mut self, value: T)\nwhere\n    // Bounds from impl:\n    A: Allocator,\n    T: Clone,"
    );
    assert_eq!(generic_impl_header(src, item_at(src, 9)), None);
    assert_eq!(generic_impl_header(src, item_at(src, 12)), None);
}

#[test]
fn test_short_type_keeps_last_path_segments() {
    assert_eq!(short_type("std::vec::Vec<shapes::Circle>"), "Vec<Circle>");
    assert_eq!(short_type("std::option::Option<&std::string::String>"), "Option<&String>");
    assert_eq!(short_type("(u8, crate::a::B)"), "(u8, B)");
}

#[test]
fn test_collapse_joins_multiline_params() {
    assert_eq!(collapse("fn f(\n    a: u8, // x\n    b: (u8, u8),\n) -> u8"), "fn f(a: u8, b: (u8, u8)) -> u8");
}

#[test]
fn test_inline_generic_bounds_move_to_where() {
    assert_eq!(with_where_layout("pub fn format<I: Integer>(&mut self, i: I) -> &str"), "pub fn format<I>(&mut self, i: I) -> &str\nwhere\n    I: Integer,");
    assert_eq!(with_where_layout("pub struct Vec<T, A: Allocator = Global>"), "pub struct Vec<T, A = Global>\nwhere\n    A: Allocator,");
    assert_eq!(with_where_layout("fn f<T: Clone, const N: usize>(t: [T; N]) where T: Send"), "fn f<T, const N: usize>(t: [T; N])\nwhere\n    T: Clone,\n    T: Send,");
    assert_eq!(with_where_layout("pub const X: Vec<u8> = Vec::new()"), "pub const X: Vec<u8> = Vec::new()");
}

#[test]
fn test_collapse_drops_attributes_and_const_bounds() {
    assert_eq!(collapse("pub struct Vec<T, #[unstable(feature = \"x\", issue = \"1\")] A: Allocator = Global>"), "pub struct Vec<T, A: Allocator = Global>");
    assert_eq!(collapse("F: [const] FnOnce(T) -> U + [const] Destruct"), "F: FnOnce(T) -> U + Destruct");
}

#[test]
fn test_impl_header_found_past_braces_in_doc_comments() {
    let src = "impl<T, A: Allocator> Vec<T, A> {\n    /// ```\n    /// for x in &v {\n    /// ```\n    pub fn len(&self) -> usize { 0 }\n    /// }\n    pub fn push(&mut self, value: T) {}\n}\n";
    assert_eq!(generic_impl_header(src, item_at(src, 7)).map(|h| h.0).as_deref(), Some("impl<T, A> Vec<T, A>"));
    let const_impl = "const impl<T, A: [const] Allocator> Vec<T, A> {\n    pub fn push(&mut self, value: T) {}\n}\n";
    assert_eq!(generic_impl_header(const_impl, item_at(const_impl, 2)).map(|h| h.0).as_deref(), Some("impl<T, A> Vec<T, A>"));
}
