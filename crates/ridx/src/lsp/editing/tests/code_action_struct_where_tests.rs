/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `rustc`-verified struct conversions (a `where` clause, field attributes) and constructors that must see only the
//! struct's own impls.

use super::code_action_assists_tests::{assert_rustc_compiles, query_deferred_and_resolve};
use super::setup_test_editing;
use ridx_test_support::apply_text_edits;

#[test]
fn test_tuple_struct_conversion_moves_the_where_clause_after_the_fields() {
    let (temp, ws, mut ov, _) = setup_test_editing();
    let path = temp.join("src/where_struct.rs");
    let uri = format!("file://{}", path.to_str().unwrap());
    let code = concat!(
        "pub struct Slot<T>\n",
        "where\n",
        "    T: Copy,\n",
        "{\n",
        "    value: T,\n",
        "    index: usize,\n",
        "}\n",
        "impl<T: Copy> Slot<T> {\n",
        "    pub fn read(&self) -> (T, usize) { (self.value, self.index) }\n",
        "    pub fn make(v: T) -> Self { Slot { value: v, index: 0 } }\n",
        "}\n",
    );
    ov.open_document(&uri, path.to_str().unwrap(), 1, code.to_string());

    let acts = query_deferred_and_resolve(&uri, 0, 12, &ws, &ov);
    let (_, edits) = acts.iter().find(|(t, _)| t == "Convert to tuple struct").expect("a where clause must not block the conversion");
    let updated = apply_text_edits(code, edits.as_array().unwrap());
    assert!(updated.contains("pub struct Slot<T>(T, usize) where\n    T: Copy;"), "{updated}");
    assert!(updated.contains("(self.0, self.1)") && updated.contains("Slot(v, 0)"), "{updated}");
    assert_rustc_compiles(&updated, "tuple struct conversion with a where clause");
}

#[test]
fn test_tuple_struct_conversion_rewrites_accesses_inside_literals_next_to_a_same_named_method() {
    let (temp, ws, mut ov, _) = setup_test_editing();
    let path = temp.join("src/node_struct.rs");
    let uri = format!("file://{}", path.to_str().unwrap());
    let code = concat!(
        "pub struct Node {\n",
        "    syntax: String,\n",
        "}\n",
        "impl Node {\n",
        "    pub fn syntax(&self) -> &String { &self.syntax }\n",
        "}\n",
        "impl Clone for Node {\n",
        "    fn clone(&self) -> Self { Self { syntax: self.syntax.clone() } }\n",
        "}\n",
        "impl PartialEq for Node {\n",
        "    fn eq(&self, other: &Self) -> bool { self.syntax == *other.syntax() }\n",
        "}\n",
    );
    ov.open_document(&uri, path.to_str().unwrap(), 1, code.to_string());

    let acts = query_deferred_and_resolve(&uri, 0, 12, &ws, &ov);
    let (_, edits) = acts.iter().find(|(t, _)| t == "Convert to tuple struct").expect("a method named like the field must not block the conversion");
    let updated = apply_text_edits(code, edits.as_array().unwrap());
    assert!(updated.contains("Self(self.0.clone())") && updated.contains("self.0 == *other.syntax()") && updated.contains("&self.0 }"), "{updated}");
    assert_rustc_compiles(&updated, "tuple struct conversion beside a method named like the field");
}

#[test]
fn test_tuple_struct_conversion_expands_struct_update_and_rest_patterns() {
    let (temp, ws, mut ov, _) = setup_test_editing();
    let path = temp.join("src/update_struct.rs");
    let uri = format!("file://{}", path.to_str().unwrap());
    let code = concat!(
        "pub struct Builder {\n",
        "    name: String,\n",
        "    size: usize,\n",
        "    depth: u8,\n",
        "}\n",
        "impl Builder {\n",
        "    pub fn size(self, size: usize) -> Self { Self { size, ..self } }\n",
        "    pub fn depth(&self) -> u8 { let Self { depth, .. } = self; *depth }\n",
        "    pub fn renamed(self, name: String) -> Self { Builder { name: name + &self.name, ..Self::new() } }\n",
        // A range in a value is not a struct update.
        "    pub fn new() -> Self { Self { name: String::new(), size: (0..1).len(), depth: 0 } }\n",
        "}\n",
    )
    .to_string();
    ov.open_document(&uri, path.to_str().unwrap(), 1, code.clone());

    // `..Self::new()` would be evaluated once per field: no conversion.
    let acts = query_deferred_and_resolve(&uri, 0, 12, &ws, &ov);
    assert!(!acts.iter().any(|(t, _)| t == "Convert to tuple struct"));

    let code = code.replace("..Self::new()", "..self");
    ov.open_document(&uri, path.to_str().unwrap(), 2, code.clone());
    let acts = query_deferred_and_resolve(&uri, 0, 12, &ws, &ov);
    let (_, edits) = acts.iter().find(|(t, _)| t == "Convert to tuple struct").expect("struct update from `self` and rest patterns convert");
    let updated = apply_text_edits(&code, edits.as_array().unwrap());
    assert!(updated.contains("Self(self.0, size, self.2)") && updated.contains("let Self(_, _, depth) = self;"), "{updated}");
    assert!(updated.contains("Builder(name + &self.0, self.1, self.2)"), "{updated}");
    assert_rustc_compiles(&updated, "tuple struct conversion with struct update and rest patterns");
}

#[test]
fn test_named_struct_conversion_keeps_field_attributes_but_not_cfg() {
    let (temp, ws, mut ov, _) = setup_test_editing();
    // A field's attribute travels with it; a `#[cfg]` would shift the positions of the fields after it.
    let path = temp.join("src/marked_conv.rs");
    let uri = format!("file://{}", path.to_str().unwrap());
    let code = concat!(
        "struct Marked(#[doc(alias = \"first\")] u8, u16);\n",
        "impl Marked { fn second(&self) -> u16 { self.1 } fn make() -> Self { Marked(1, 2) } }\n",
        "struct Gated(#[cfg(test)] u8, u16);\n",
    );
    ov.open_document(&uri, path.to_str().unwrap(), 1, code.to_string());
    let acts = query_deferred_and_resolve(&uri, 0, 8, &ws, &ov);
    let (_, edits) = acts.iter().find(|(t, _)| t == "Convert to named struct").expect("tuple field attribute converts");
    let updated = apply_text_edits(code, edits.as_array().unwrap());
    assert!(updated.contains("struct Marked { #[doc(alias = \"first\")] field1: u8, field2: u16 }") && updated.contains("self.field2 }"), "{updated}");
    assert_rustc_compiles(&updated, "tuple field attribute to named struct");
    assert!(!query_deferred_and_resolve(&uri, 2, 8, &ws, &ov).iter().any(|(t, _)| t == "Convert to named struct"), "cfg'd tuple field");
}

#[test]
fn test_generate_new_and_single_field_from_see_only_this_structs_impls() {
    let (temp, ws, mut ov, _) = setup_test_editing();
    let path = temp.join("src/ctor_scope.rs");
    let uri = format!("file://{}", path.to_str().unwrap());
    let code = concat!(
        "fn a() -> u32 {\n",
        "    struct Size { w: u32 }\n",
        "    impl Size { fn new(w: u32) -> Self { Size { w } } }\n",
        "    Size::new(1).w\n",
        "}\n",
        "fn b() -> u32 {\n",
        "    struct Size { w: u32 }\n",
        "    Size { w: 1 }.w\n",
        "}\n",
        "macro_rules! wrap { ($($t:tt)*) => { $($t)* }; }\n",
        "pub struct Options { pub limit: usize }\n",
        "wrap! { pub fn opts() -> Box<Options> { Box::new(Options { limit: a() as usize + b() as usize }) } }\n",
        "mod other { pub struct MatchKind(pub u8); }\n",
        "pub struct MatchKind { pub kind: other::MatchKind }\n",
        "pub struct Opt { pub arg: bool, pub no_arg: () }\n",
    );
    ov.open_document(&uri, path.to_str().unwrap(), 1, code.to_string());
    let edit_at = |line: u32, col: u32, title: &str| {
        let acts = query_deferred_and_resolve(&uri, line, col, &ws, &ov);
        acts.iter().find(|(t, _)| t == title).map(|(_, e)| apply_text_edits(code, e.as_array().unwrap()))
    };

    // `a`'s `Size` has its `new`; `b`'s is another struct; `Box::new` in a macro call defines nothing.
    assert!(edit_at(1, 11, "Generate `new`").is_none(), "a::Size already has new");
    for (line, col, what) in [(6u32, 11u32, "b::Size"), (10, 12, "Options")] {
        let updated = edit_at(line, col, "Generate `new`").unwrap_or_else(|| panic!("Generate `new` for {what}"));
        assert_rustc_compiles(&updated, &format!("Generate `new` for {what}"));
    }
    // A field type reached through another module is another type; a `()` field is built as `()`.
    let kind = edit_at(13, 12, "Generate single field `From`").expect("From<other::MatchKind>");
    assert!(kind.contains("impl From<other::MatchKind> for MatchKind"), "{kind}");
    assert_rustc_compiles(&kind, "single field From of a same-named type");
    let opt = edit_at(14, 12, "Generate single field `From`").expect("From<bool> with a unit field");
    assert!(opt.contains("Self { arg, no_arg: () }"), "{opt}");
    assert_rustc_compiles(&opt, "single field From with a unit field");
}
