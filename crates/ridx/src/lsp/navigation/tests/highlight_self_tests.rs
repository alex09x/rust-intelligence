/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `Self` in document highlights of a type: inside each impl whose header the index resolved to the type, also a
//! qualified header and a header with a `{ … }` const argument; not inside an impl nested in one of its methods.

use super::super::highlight_ext::augment_resolved_highlights;
use crate::lsp::protocol::PositionConverter;
use ridx_format::DefKind;
use serde_json::{json, Value};
use std::collections::BTreeSet;

const SRC: &str = "pub struct Foo;
impl crate::Foo {
    fn a() -> Self { Self }
}
impl<const N: usize> Foo<{ 1 }> {
    fn b() -> Self { todo!() }
}
impl Foo {
    fn c() {
        struct Other;
        impl Other { fn d() -> Self { Other } }
        let _ = Self::a();
    }
}
";

fn hl(line: usize, col: usize) -> Value {
    json!({ "range": { "start": { "line": line, "character": col }, "end": { "line": line, "character": col + 3 } }, "kind": 1 })
}

/// `(line, column)` of every `needle` on `line`.
fn all_on(line: usize, needle: &str) -> Vec<(u64, u64)> {
    let text = SRC.lines().nth(line).unwrap();
    text.match_indices(needle).map(|(c, _)| (line as u64, c as u64)).collect()
}

#[test]
fn self_follows_the_impls_of_the_target_only() {
    let dir = tempfile::TempDir::new().unwrap();
    let file = dir.path().join("lib.rs");
    std::fs::write(&file, SRC).unwrap();
    let file = file.to_string_lossy().into_owned();
    // What the index resolves: the declaration and the three headers' `Foo`.
    let mut highlights: Vec<Value> = [(0, 11), (1, 12), (4, 21), (7, 5)].into_iter().map(|(l, c)| hl(l, c)).collect();
    let mut conv = PositionConverter::new();
    augment_resolved_highlights(&file, SRC, ("Foo", "Foo", "demo"), DefKind::Struct, &mut conv, &mut highlights);
    let got: BTreeSet<(u64, u64)> = highlights.iter().map(|h| (h["range"]["start"]["line"].as_u64().unwrap(), h["range"]["start"]["character"].as_u64().unwrap())).collect();
    let mut want: BTreeSet<(u64, u64)> = [(0, 11), (1, 12), (4, 21), (7, 5)].into_iter().collect();
    want.extend(all_on(2, "Self"));
    want.extend(all_on(5, "Self"));
    want.extend(all_on(11, "Self"));
    assert_eq!(got, want, "`Self` in `impl Other` (line 10) is Other");
}
