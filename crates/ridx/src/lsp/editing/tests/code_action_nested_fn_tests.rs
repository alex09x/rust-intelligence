/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `rustc`-verified tests for `Convert nested function to closure` used in std macros and gated by `#[cfg]`.

use super::code_action_assists_tests::{assert_rustc_compiles, query_deferred_and_resolve};
use super::setup_test_editing;
use ridx_test_support::apply_text_edits;

const TITLE: &str = "Convert nested function to closure";

#[test]
fn test_nested_fn_called_in_assert_and_cfg_twins_become_closures() {
    let (temp, ws, mut ov, _) = setup_test_editing();
    let path = temp.join("src/nested_cfg.rs");
    let uri = format!("file://{}", path.to_str().unwrap());
    let code = concat!(
        "pub fn run(m: Option<u8>) -> u8 {\n",
        "    fn is_some(m: Option<u8>) -> bool {\n",
        "        match m {\n",
        "            Some(_) => true,\n",
        "            _ => false,\n",
        "        }\n",
        "    }\n",
        "    assert!(is_some(m) || m.is_none());\n",
        "    #[cfg(not(all()))]\n",
        "    fn imp(_: &'static str) -> Result<u8, ()> {\n",
        "        Err(())\n",
        "    }\n",
        "    #[cfg(all())]\n",
        "    fn imp(name: &'static str) -> Result<u8, ()> {\n",
        "        Ok(name.len() as u8)\n",
        "    }\n",
        "    imp(\"ab\").unwrap_or(0)\n",
        "}\n",
    );
    ov.open_document(&uri, path.to_str().unwrap(), 1, code.to_string());

    // A call inside `assert!` is an ordinary run-time call; each cfg twin keeps its `#[cfg]` on the `let`.
    for (line, col, expect) in [
        (1, 8, "let is_some = |m: Option<u8>| -> bool"),
        (13, 8, "#[cfg(all())]\n    let imp = |name: &'static str|"),
        (9, 8, "#[cfg(not(all()))]\n    let imp = |_: &'static str|"),
    ] {
        let acts = query_deferred_and_resolve(&uri, line, col, &ws, &ov);
        let (_, edits) = acts.iter().find(|(t, _)| t == TITLE).unwrap_or_else(|| panic!("line {line}: {TITLE} offered in {:?}", acts.iter().map(|(t, _)| t).collect::<Vec<_>>()));
        let updated = apply_text_edits(code, edits.as_array().unwrap());
        assert!(updated.contains(expect), "{updated}");
        assert_rustc_compiles(&updated, TITLE);
    }
}

#[test]
fn test_nested_fn_under_cfg_attr_or_in_a_local_macro_stays() {
    let (temp, ws, mut ov, _) = setup_test_editing();
    let path = temp.join("src/nested_keep.rs");
    let uri = format!("file://{}", path.to_str().unwrap());
    let code = concat!(
        "pub fn run() -> u8 {\n",
        "    macro_rules! assert { ($e:expr) => { const _: u8 = $e; }; }\n",
        "    fn one() -> u8 { 1 }\n",
        "    assert!(one());\n",
        "    #[cfg_attr(all(), inline)]\n",
        "    fn two() -> u8 { 2 }\n",
        "    one() + two()\n",
        "}\n",
    );
    ov.open_document(&uri, path.to_str().unwrap(), 1, code.to_string());
    for line in [2, 5] {
        let acts = query_deferred_and_resolve(&uri, line, 8, &ws, &ov);
        assert!(!acts.iter().any(|(t, _)| t == TITLE), "line {line}");
    }
}
