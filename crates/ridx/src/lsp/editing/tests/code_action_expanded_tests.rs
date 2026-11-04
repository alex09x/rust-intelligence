/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `rustc`-verified tests for clean doc templates, receiver-preserving method aliases, string/extract assists, and ADT/inline assists.

use super::code_action_assists_tests::{assert_rustc_compiles, query_deferred_and_resolve};
use super::setup_test_editing;
use ridx_test_support::apply_text_edits;

#[test]
fn test_method_type_alias_preserves_receiver_and_coerces_method_item() {
    let (temp, ws, mut ov, _) = setup_test_editing();
    let path = temp.join("src/method_coercion.rs");
    let uri = format!("file://{}", path.to_str().unwrap());
    let code = concat!(
        "use std::pin::Pin;\n",
        "use std::task::{Context, Poll};\n",
        "pub struct PipeClient;\n",
        "impl PipeClient {\n",
        "    pub fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {\n",
        "        Poll::Ready(Ok(()))\n",
        "    }\n",
        "}\n",
    );
    ov.open_document(&uri, path.to_str().unwrap(), 1, code.to_string());

    let actions = query_deferred_and_resolve(&uri, 4, 12, &ws, &ov);
    let (_, named_edits) = actions.iter().find(|(t, _)| t == "Generate a type alias for function with named params").expect("named alias must exist");
    let updated = apply_text_edits(code, named_edits.as_array().unwrap());
    assert!(updated.contains("Pin<&mut PipeClient>"), "Receiver parameter must be preserved in method fn type alias: {updated}");
    let with_coercion = format!("{updated}\npub fn bind_method_ptr() -> PollReadyFn {{ PipeClient::poll_ready }}\n");
    assert_rustc_compiles(&with_coercion, "method item coerces to generated PollReadyFn");
}

#[test]
fn test_doc_template_sections_getter_new_and_doc_hidden() {
    let (temp, ws, mut ov, _) = setup_test_editing();
    let path = temp.join("src/doc_sections.rs");
    let uri = format!("file://{}", path.to_str().unwrap());
    let code = concat!(
        "pub struct Buffer<T> { items: Vec<T> }\n",
        "impl<T> Buffer<T> {\n",
        "    pub fn new() -> Self { Self { items: Vec::new() } }\n",
        "    pub fn items(&self) -> &[T] { &self.items }\n",
        "}\n",
        "#[doc(hidden)]\n",
        "pub unsafe fn parse_checked(input: &str) -> Result<usize, ()> {\n",
        "    assert!(!input.is_empty());\n",
        "    Ok(input.len())\n",
        "}\n",
    );
    ov.open_document(&uri, path.to_str().unwrap(), 1, code.to_string());

    let new_acts = query_deferred_and_resolve(&uri, 2, 11, &ws, &ov);
    let (_, new_ed) = new_acts.iter().find(|(t, _)| t == "Generate a documentation template").unwrap();
    let u_new = apply_text_edits(code, new_ed.as_array().unwrap());
    assert!(u_new.contains("/// Creates a new [`Buffer<T>`]."));
    assert_rustc_compiles(&u_new, "doc template on new()");

    let get_acts = query_deferred_and_resolve(&uri, 3, 11, &ws, &ov);
    let (_, get_ed) = get_acts.iter().find(|(t, _)| t == "Generate a documentation template").unwrap();
    let u_get = apply_text_edits(code, get_ed.as_array().unwrap());
    assert!(u_get.contains("/// Returns a reference to the items of this [`Buffer<T>`]."));
    assert_rustc_compiles(&u_get, "doc template on getter");

    let sec_acts = query_deferred_and_resolve(&uri, 6, 15, &ws, &ov);
    let (_, sec_ed) = sec_acts.iter().find(|(t, _)| t == "Generate a documentation template").unwrap();
    let u_sec = apply_text_edits(code, sec_ed.as_array().unwrap());
    assert!(u_sec.contains("/// .\n///\n/// # Panics\n///\n/// Panics if .\n///\n/// # Errors\n///\n/// This function will return an error if .\n///\n/// # Safety\n///\n/// ."));
    assert_rustc_compiles(&u_sec, "doc template with Panics/Errors/Safety");
}

#[test]
fn test_raw_string_and_extract_literal_assists_compile() {
    let (temp, ws, mut ov, _) = setup_test_editing();
    let path = temp.join("src/str_lit.rs");
    let uri = format!("file://{}", path.to_str().unwrap());
    let code =
        concat!("fn check_fixture(ra_fixture_after: &str) -> usize { ra_fixture_after.len() }\n", "pub fn run() -> usize {\n", "    check_fixture(r#\"hello\nworld\"#)\n", "}\n",);
    ov.open_document(&uri, path.to_str().unwrap(), 1, code.to_string());

    let acts = query_deferred_and_resolve(&uri, 2, 22, &ws, &ov);
    let titles: Vec<&str> = acts.iter().map(|(t, _)| t.as_str()).collect();
    for expected in ["Add #", "Remove #", "Rewrite as regular string", "Extract into variable", "Extract into constant", "Extract into static"] {
        assert!(titles.contains(&expected), "Missing '{expected}' in {titles:?}");
    }
    for (title, edits) in &acts {
        let updated = apply_text_edits(code, edits.as_array().unwrap());
        if title == "Extract into variable" {
            assert!(updated.contains("let ra_fixture_after ="), "Should infer callee param name: {updated}");
        } else if title == "Extract into constant" {
            assert!(updated.contains("const RA_FIXTURE_AFTER: &str ="), "Should infer UPPER_SNAKE param name: {updated}");
        }
        assert_rustc_compiles(&updated, title);
    }
}

#[test]
fn test_struct_conversions_single_from_default_and_enum_discriminants() {
    let (temp, ws, mut ov, _) = setup_test_editing();
    let path = temp.join("src/adt_expanded.rs");
    let uri = format!("file://{}", path.to_str().unwrap());
    let code = concat!(
        "struct Offset { val: i32 }\n",
        "impl Offset {\n",
        "    pub fn new() -> Self { Self { val: 0 } }\n",
        "    pub fn get(&self) -> i32 { self.val }\n",
        "}\n",
        "struct TupleWrap(u64);\n",
        "impl TupleWrap {\n",
        "    pub fn read(&self) -> u64 { self.0 }\n",
        "    pub fn make(x: u64) -> Self { TupleWrap(x) }\n",
        "}\n",
        "pub struct GenericWrap<T>(pub T);\n",
        "pub enum Mode { Fast, Balanced, Accurate }\n",
    );
    ov.open_document(&uri, path.to_str().unwrap(), 1, code.to_string());

    // 1. Offset: Convert to tuple struct & Generate single field `From`.
    let off_acts = query_deferred_and_resolve(&uri, 0, 8, &ws, &ov);
    assert!(off_acts.iter().any(|(t, _)| t == "Convert to tuple struct"));
    assert!(off_acts.iter().any(|(t, _)| t == "Generate single field `From`"));
    for (title, edits) in &off_acts {
        assert_rustc_compiles(&apply_text_edits(code, edits.as_array().unwrap()), title);
    }

    // 2. Offset::new: Generate a Default impl from a new fn.
    let new_acts = query_deferred_and_resolve(&uri, 2, 11, &ws, &ov);
    assert!(new_acts.iter().any(|(t, _)| t == "Generate a Default impl from a new fn"));
    for (title, edits) in &new_acts {
        assert_rustc_compiles(&apply_text_edits(code, edits.as_array().unwrap()), title);
    }

    // 3. TupleWrap: Convert to named struct.
    let tup_acts = query_deferred_and_resolve(&uri, 5, 8, &ws, &ov);
    assert!(tup_acts.iter().any(|(t, _)| t == "Convert to named struct"));
    for (title, edits) in &tup_acts {
        assert_rustc_compiles(&apply_text_edits(code, edits.as_array().unwrap()), title);
    }

    // 4. GenericWrap<T>(T): bare `T` must NOT offer conflicting `From<T>` (`E0119`).
    let gen_acts = query_deferred_and_resolve(&uri, 10, 12, &ws, &ov);
    assert!(!gen_acts.iter().any(|(t, _)| t == "Generate single field `From`"), "Bare T must reject E0119 conflicting From<T>");

    // 5. Mode: Add explicit enum discriminants.
    let enum_acts = query_deferred_and_resolve(&uri, 11, 10, &ws, &ov);
    let (_, disc_ed) = enum_acts.iter().find(|(t, _)| t == "Add explicit enum discriminants").expect("enum discriminants");
    let u_enum = apply_text_edits(code, disc_ed.as_array().unwrap());
    assert!(u_enum.contains("Fast = 0, Balanced = 1, Accurate = 2"));
    assert_rustc_compiles(&u_enum, "Add explicit enum discriminants");
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

#[test]
fn test_compile_safety_on_chained_tuple_proj_binary_inline_try_from_const_fn_and_missing_docs() {
    let (temp, ws, mut ov, _) = setup_test_editing();
    let path = temp.join("src/edge_safety.rs");
    let uri = format!("file://{}", path.to_str().unwrap());
    let code = concat!(
        "//! Crate doc.\n",
        "#![deny(missing_docs)]\n",
        "/// Documented struct.\n",
        "pub struct SizeErr {\n",
        "    len: usize,\n",
        "}\n",
        "/// Documented wrapper.\n",
        "pub struct UnitVec(i32);\n",
        "impl core::convert::TryFrom<i32> for UnitVec {\n",
        "    type Error = ();\n",
        "    fn try_from(v: i32) -> Result<Self, ()> { Ok(Self(v)) }\n",
        "}\n",
        "fn is_pos(x: i32) -> bool { match x { n if n > 0 => true, _ => false } }\n",
        "/// Entry.\n",
        "pub fn check_all(v: i32) -> bool {\n",
        "    struct Guard(String);\n",
        "    impl Drop for Guard {\n",
        "        fn drop(&mut self) { let _ = self.0.len(); }\n",
        "    }\n",
        "    const fn c_id(n: i32) -> i32 { n }\n",
        "    fn helper(n: i32) -> i32 { n + 1 }\n",
        "    const _F: fn(i32) -> i32 = helper;\n",
        "    let _g = Guard(String::new());\n",
        "    let _ = c_id(v);\n",
        "    is_pos(v) || is_pos(-v)\n",
        "}\n",
    );
    ov.open_document(&uri, path.to_str().unwrap(), 1, code.to_string());

    // 1. `#![deny(missing_docs)]` + `Generate new` emits doc comment without `#[allow]` and passes `rustc`.
    let new_acts = query_deferred_and_resolve(&uri, 3, 12, &ws, &ov);
    let (_, new_ed) = new_acts.iter().find(|(t, _)| t == "Generate `new`").expect("Generate new");
    let u_new = apply_text_edits(code, new_ed.as_array().unwrap());
    assert!(u_new.contains("/// Creates a new [`SizeErr`].") && !u_new.contains("#[allow("));
    assert_rustc_compiles(&u_new, "Generate `new` under #![deny(missing_docs)]");

    // 2. Existing `TryFrom<i32> for UnitVec` blocks conflicting `Generate single field From` (`E0119`).
    let uv_acts = query_deferred_and_resolve(&uri, 7, 12, &ws, &ov);
    assert!(!uv_acts.iter().any(|(t, _)| t == "Generate single field `From`"));

    // 3. `Inline into all callers` parenthesizes `{ ... } || { ... }` so `rustc` does not parse closure `||`.
    let inl_acts = query_deferred_and_resolve(&uri, 12, 5, &ws, &ov);
    let (_, inl_ed) = inl_acts.iter().find(|(t, _)| t == "Inline into all callers").expect("Inline into all callers");
    assert_rustc_compiles(&apply_text_edits(code, inl_ed.as_array().unwrap()), "Inline into binary ||");

    // 4. `Convert to named struct` rewrites `self.0.len()` chained tuple projection cleanly.
    let gd_acts = query_deferred_and_resolve(&uri, 15, 12, &ws, &ov);
    let (_, gd_ed) = gd_acts.iter().find(|(t, _)| t == "Convert to named struct").expect("Convert to named struct");
    let u_gd = apply_text_edits(code, gd_ed.as_array().unwrap());
    assert!(u_gd.contains("self.field1.len()"));
    assert_rustc_compiles(&u_gd, "Convert to named struct with chained .0.len()");

    // 5. Nested `fn helper` referenced inside `const _F` refuses `Convert nested function to closure` (`E0435`).
    let fn_acts = query_deferred_and_resolve(&uri, 20, 8, &ws, &ov);
    assert!(!fn_acts.iter().any(|(t, _)| t == "Convert nested function to closure"));

    // 6. Function returning `impl Trait` refuses `Inline into all callers` to prevent E0283 inference regressions.
    let impl_trait_code = concat!("fn gen_iter() -> impl Iterator<Item = i32> { std::iter::empty() }\n", "fn caller() { for _ in gen_iter() {} }\n",);
    let impl_path = temp.join("src/impl_trait_inline.rs");
    let impl_uri = format!("file://{}", impl_path.to_str().unwrap());
    ov.open_document(&impl_uri, impl_path.to_str().unwrap(), 1, impl_trait_code.to_string());
    let impl_acts = query_deferred_and_resolve(&impl_uri, 0, 5, &ws, &ov);
    assert!(!impl_acts.iter().any(|(t, _)| t == "Inline into all callers"));
}
