/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The macros an inlined body calls must be in scope in the file it moves to: a `macro_rules!` of the function's file
//! is not, unless that file already calls it.

use super::action_inline_callers_qualify::tokens;
use super::action_parse::{is_ident_byte, mask_comments_and_strings};

/// Macros every expansion site sees wherever it is (`vec!`, `write!`, …).
pub(crate) const STD_MACROS: &[&str] = &[
    "assert",
    "assert_eq",
    "assert_ne",
    "cfg",
    "column",
    "compile_error",
    "concat",
    "dbg",
    "debug_assert",
    "debug_assert_eq",
    "debug_assert_ne",
    "env",
    "eprint",
    "eprintln",
    "file",
    "format",
    "format_args",
    "include",
    "include_bytes",
    "include_str",
    "line",
    "matches",
    "module_path",
    "option_env",
    "panic",
    "print",
    "println",
    "stringify",
    "todo",
    "unimplemented",
    "unreachable",
    "vec",
    "write",
    "writeln",
];

/// Whether each macro `body` calls is a standard one, named by path (`crate::m!`), or already called in `dest`.
pub(crate) fn macros_in_scope(body: &str, dest: &str) -> bool {
    let masked = mask_comments_and_strings(body);
    let dest_masked = mask_comments_and_strings(dest);
    tokens(&masked).into_iter().all(|(i, e)| {
        let tok = &masked[i..e];
        let bang = masked[e..].starts_with('!') && !masked[e..].starts_with("!=");
        !bang
            || masked[..i].ends_with("::")
            || STD_MACROS.contains(&tok)
            || dest_masked.match_indices(tok).any(|(p, _)| dest_masked[p + tok.len()..].starts_with('!') && (p == 0 || !is_ident_byte(dest_masked.as_bytes()[p - 1])))
    })
}

#[cfg(test)]
mod tests {
    use super::macros_in_scope;

    #[test]
    fn a_local_macro_must_already_be_called_in_the_destination() {
        let body = "if x != 1 { return err!(\"bad\"); } vec![x]; crate::fail!()";
        assert!(!macros_in_scope(body, "fn g() { let v = vec![1]; }"));
        assert!(macros_in_scope(body, "fn g() -> R { err!(\"other\") }"));
    }
}
