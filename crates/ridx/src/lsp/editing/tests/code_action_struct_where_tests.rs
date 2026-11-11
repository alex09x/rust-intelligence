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
