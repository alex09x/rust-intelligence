/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `rustc`-verified tests for `Extract into variable / constant / static` on a literal that is a whole initializer.

use super::code_action_assists_tests::{assert_rustc_compiles, query_deferred_and_resolve};
use super::setup_test_editing;
use ridx_test_support::apply_text_edits;

const EXTRACTS: [&str; 3] = ["Extract into variable", "Extract into constant", "Extract into static"];

#[test]
fn test_literal_bound_by_let_is_extracted_like_rust_analyzer() {
    let (temp, ws, mut ov, _) = setup_test_editing();
    let path = temp.join("src/let_fixture.rs");
    let uri = format!("file://{}", path.to_str().unwrap());
    let code = concat!(
        "fn check(fixture: &str) -> usize { fixture.len() }\n",
        "pub fn run() -> usize {\n",
        "    let code = r#\"\n",
        "        struct A {}\n",
        "    \"#;\n",
        "    let bytes: &[u8; 3] = b\"abc\";\n",
        "    const NAME: &str = \"name\";\n",
        "    check(code) + bytes.len() + NAME.len()\n",
        "}\n",
    );
    ov.open_document(&uri, path.to_str().unwrap(), 1, code.to_string());

    // A raw string that is the whole `let` initializer: all three extractions, each compiling.
    let acts = query_deferred_and_resolve(&uri, 3, 12, &ws, &ov);
    for title in EXTRACTS {
        let (_, edits) = acts.iter().find(|(t, _)| t == title).unwrap_or_else(|| panic!("{title} offered: {acts:?}"));
        let updated = apply_text_edits(code, edits.as_array().unwrap());
        assert_rustc_compiles(&updated, title);
    }

    // A byte string under a typed `let` (`&[u8; 3]`) and a local `const` initializer are left alone.
    for (line, col) in [(5, 29), (6, 25)] {
        let acts = query_deferred_and_resolve(&uri, line, col, &ws, &ov);
        assert!(!acts.iter().any(|(t, _)| EXTRACTS.contains(&t.as_str())), "line {line}: {:?}", acts.iter().map(|(t, _)| t).collect::<Vec<_>>());
    }
}
