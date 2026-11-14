/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `rustc`-verified tests for tuple-struct conversion and the nested fn, closure, alias and caller inlining assists.

use super::code_action_assists_tests::{assert_rustc_compiles, query_deferred_and_resolve};
use super::setup_test_editing;
use ridx_test_support::apply_text_edits;

#[test]
fn test_tuple_struct_conversion_keeps_comments_between_fields() {
    let (temp, ws, mut ov, _) = setup_test_editing();
    let path = temp.join("src/commented_conv.rs");
    let uri = format!("file://{}", path.to_str().unwrap());
    let code = concat!(
        "#[derive(Default, Debug)]\n",
        "struct Ctxt {\n",
        "    // Set to `None` while checking.\n",
        "    errors: Option<Vec<String>>,\n",
        "    /// How many were seen.\n",
        "    seen: usize,\n",
        "}\n",
        "impl Ctxt {\n",
        "    fn new() -> Self { Ctxt { errors: Some(Vec::new()), seen: 0 } }\n",
        "    fn count(&self) -> usize { self.seen + self.errors.as_ref().map_or(0, Vec::len) }\n",
        "}\n",
        "pub fn run() -> usize { Ctxt::new().count() }\n",
    );
    ov.open_document(&uri, path.to_str().unwrap(), 1, code.to_string());

    let acts = query_deferred_and_resolve(&uri, 1, 8, &ws, &ov);
    let (_, edits) = acts.iter().find(|(t, _)| t == "Convert to tuple struct").expect("comments between fields must not block the conversion");
    let updated = apply_text_edits(code, edits.as_array().unwrap());
    assert!(updated.contains("    // Set to `None` while checking.\n    Option<Vec<String>>,\n    /// How many were seen.\n    usize,\n);"), "{updated}");
    assert!(updated.contains("self.1 + self.0.as_ref()"), "{updated}");
    assert_rustc_compiles(&updated, "tuple struct conversion with comments between fields");
}

#[test]
fn test_tuple_struct_conversion_beside_a_trait_method_named_like_the_field() {
    let (temp, ws, mut ov, _) = setup_test_editing();
    let path = temp.join("src/method_named_field.rs");
    let uri = format!("file://{}", path.to_str().unwrap());
    let code = concat!(
        "pub struct Rename {\n",
        "    syntax: u32,\n",
        "}\n",
        "pub trait AstNode { fn syntax(&self) -> &u32; }\n",
        "impl AstNode for Rename {\n",
        "    fn syntax(&self) -> &u32 { &self.syntax }\n",
        "}\n",
    );
    ov.open_document(&uri, path.to_str().unwrap(), 1, code.to_string());

    // rust-analyzer's `ast::Rename`: `.syntax` is the field, `.syntax()` the method; only the field is rewritten.
    let acts = query_deferred_and_resolve(&uri, 0, 12, &ws, &ov);
    let (_, edits) = acts.iter().find(|(t, _)| t == "Convert to tuple struct").expect("the method does not block the conversion");
    let updated = apply_text_edits(code, edits.as_array().unwrap());
    assert!(updated.contains("pub struct Rename(u32);") && updated.contains("&self.0 }"), "{updated}");
    assert_rustc_compiles(&updated, "tuple struct conversion beside a same-named trait method");
}

#[test]
fn test_tuple_struct_conversion_leaves_same_named_variant_alone() {
    let (temp, ws, mut ov, _) = setup_test_editing();
    let path = temp.join("src/variant_conv.rs");
    let uri = format!("file://{}", path.to_str().unwrap());
    let code = concat!(
        "pub enum Diag { TypedHole { expr: u32 } }\n",
        "struct TypedHole { expr: u32 }\n",
        "fn lower(d: &Diag) -> TypedHole {\n",
        "    match d {\n",
        "        Diag::TypedHole { expr } => TypedHole { expr: *expr },\n",
        "    }\n",
        "}\n",
        "pub fn run() -> u32 {\n",
        "    let t: TypedHole = lower(&Diag::TypedHole { expr: 1 });\n",
        "    t.expr\n",
        "}\n",
    );
    ov.open_document(&uri, path.to_str().unwrap(), 1, code.to_string());

    // Without the index (this buffer is not indexed) the variant's declaration and patterns look like the struct's
    // literals, so the conversion is refused rather than rewriting the enum.
    let acts = query_deferred_and_resolve(&uri, 1, 9, &ws, &ov);
    assert!(!acts.iter().any(|(t, _)| t == "Convert to tuple struct"), "{:?}", acts.iter().map(|(t, _)| t).collect::<Vec<_>>());
}

#[test]
fn test_nested_fn_closure_inline_type_alias_and_inline_callers() {
    let (temp, ws, mut ov, _) = setup_test_editing();
    let path = temp.join("src/inline_cases.rs");
    let uri = format!("file://{}", path.to_str().unwrap());
    let code = concat!(
        "type Pair<'a> = (&'a str, usize);\n",
        "fn combine(x: usize, y: usize) -> usize { x + y * 2 }\n",
        "pub fn entry(s: &str) -> Pair<'_> {\n",
        "    fn scale(v: usize) -> usize { v * 3 }\n",
        "    let x = 4usize;\n",
        "    let n = combine(10, x) + scale(2);\n",
        "    (s, n)\n",
        "}\n",
    );
    ov.open_document(&uri, path.to_str().unwrap(), 1, code.to_string());

    // 1. Inline type alias into all uses.
    let alias_acts = query_deferred_and_resolve(&uri, 0, 6, &ws, &ov);
    assert!(alias_acts.iter().any(|(t, _)| t == "Inline type alias into all uses"));
    for (title, edits) in &alias_acts {
        assert_rustc_compiles(&apply_text_edits(code, edits.as_array().unwrap()), title);
    }

    // 2. Inline into all callers (verifies simultaneous tuple parameter binding when caller also has `x`).
    let fn_acts = query_deferred_and_resolve(&uri, 1, 5, &ws, &ov);
    assert!(fn_acts.iter().any(|(t, _)| t == "Inline into all callers"));
    for (title, edits) in &fn_acts {
        assert_rustc_compiles(&apply_text_edits(code, edits.as_array().unwrap()), title);
    }

    // 3. Convert nested function to closure.
    let nested_acts = query_deferred_and_resolve(&uri, 3, 8, &ws, &ov);
    assert!(nested_acts.iter().any(|(t, _)| t == "Convert nested function to closure"));
    for (title, edits) in &nested_acts {
        let updated = apply_text_edits(code, edits.as_array().unwrap());
        if title == "Convert nested function to closure" {
            assert!(updated.contains("let scale = |v: usize| -> usize { v * 3 };"));
        }
        assert_rustc_compiles(&updated, title);
    }
}
