/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `rustc`-verified `Convert to tuple struct` on a struct with a `where` clause.

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
