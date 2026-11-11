/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `Implement default members` copies an associated const's default as it does a default method.

use super::code_action_assists_tests::{assert_rustc_compiles, query_deferred_and_resolve};
use super::setup_test_editing;
use ridx_test_support::apply_text_edits;

#[test]
fn test_default_members_implement_an_associated_const_default() {
    let (temp, ws, mut ov, _) = setup_test_editing();
    let path = temp.join("src/assoc_const_default.rs");
    let uri = format!("file://{}", path.to_str().unwrap());
    let code = concat!(
        "pub trait PhaseItem {\n",
        "    const AUTOMATIC_BATCHING: bool = true;\n",
        "    const SLOT: u32;\n",
        "    fn entity(&self) -> u32;\n",
        "}\n",
        "pub struct Opaque;\n",
        "impl PhaseItem for Opaque {\n",
        "    const SLOT: u32 = 1;\n",
        "    fn entity(&self) -> u32 { 0 }\n",
        "}\n",
        "pub struct Done;\n",
        "impl PhaseItem for Done {\n",
        "    const AUTOMATIC_BATCHING: bool = false;\n",
        "    const SLOT: u32 = 2;\n",
        "    fn entity(&self) -> u32 { 0 }\n",
        "}\n",
    );
    ov.open_document(&uri, path.to_str().unwrap(), 1, code.to_string());
    let actions = query_deferred_and_resolve(&uri, 8, 8, &ws, &ov);
    let (_, edits) = actions.iter().find(|(t, _)| t == "Implement default members").expect("the const default is a member to implement");
    let updated = apply_text_edits(code, edits.as_array().unwrap());
    assert_eq!(updated.matches("const AUTOMATIC_BATCHING: bool = true;").count(), 2, "the trait's default copied into the impl: {updated}");
    assert_rustc_compiles(&updated, "Implement default members");
    let done = query_deferred_and_resolve(&uri, 14, 8, &ws, &ov);
    assert!(!done.iter().any(|(t, _)| t == "Implement default members"), "an implemented const is not offered again");
}
