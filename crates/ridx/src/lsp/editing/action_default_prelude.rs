/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Prelude names a copied default member relies on (`Result`, `Vec`, …) that the impl's module shadows: they are
//! written as their full paths so they keep the meaning they had in the trait's file.

use super::action_parse::is_ident_byte;
use super::action_parse_decl::next_ident_token;
use std::collections::HashSet;

const CORE: &[(&str, &str)] = &[
    ("Result", "::core::result::Result"),
    ("Option", "::core::option::Option"),
    ("Some", "::core::option::Option::Some"),
    ("None", "::core::option::Option::None"),
    ("Ok", "::core::result::Result::Ok"),
    ("Err", "::core::result::Result::Err"),
    ("Default", "::core::default::Default"),
    ("Iterator", "::core::iter::Iterator"),
    ("IntoIterator", "::core::iter::IntoIterator"),
    ("From", "::core::convert::From"),
    ("Into", "::core::convert::Into"),
];

const STD: &[(&str, &str)] = &[
    ("Box", "::std::boxed::Box"),
    ("Vec", "::std::vec::Vec"),
    ("String", "::std::string::String"),
    ("ToString", "::std::string::ToString"),
    ("ToOwned", "::std::borrow::ToOwned"),
];

/// Whether the member body names a prelude item that `scope` (the impl module's own items and imports) shadows.
pub(crate) fn shadows_prelude_name(body_masked: &str, scope: &HashSet<String>) -> bool {
    CORE.iter().chain(STD).any(|(p, _)| scope.contains(*p) && super::action_parse_items::contains_word(body_masked, p))
}

/// `src` (masked as `masked`) with each prelude name that `scope` shadows (`type Result<T> = …` in the impl's module)
/// written as its full path; `None` when such a name lives in `alloc` and the impl's crate does not link `std`.
pub(crate) fn unshadow_prelude(src: &str, masked: &str, scope: &HashSet<String>, dest_std: bool) -> Option<String> {
    let b = masked.as_bytes();
    let mut edits: Vec<(usize, usize, &str)> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if (b[i].is_ascii_alphabetic() || b[i] == b'_') && (i == 0 || !is_ident_byte(b[i - 1])) {
            if let Some((tok, end)) = next_ident_token(masked, i) {
                let before = masked[..i].trim_end();
                if scope.contains(tok) && !before.ends_with("::") && !before.ends_with('.') {
                    if let Some(&(_, p)) = CORE.iter().find(|(n, _)| *n == tok) {
                        edits.push((i, end, p));
                    } else if let Some(&(_, p)) = STD.iter().find(|(n, _)| *n == tok) {
                        if !dest_std {
                            return None;
                        }
                        edits.push((i, end, p));
                    }
                }
                i = end;
                continue;
            }
        }
        i += 1;
    }
    let mut out = src.to_string();
    for (lo, hi, p) in edits.into_iter().rev() {
        out.replace_range(lo..hi, p);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::unshadow_prelude;
    use std::collections::HashSet;

    #[test]
    fn shadowed_prelude_names_are_written_in_full() {
        let scope: HashSet<String> = ["Result".to_string(), "Vec".to_string()].into_iter().collect();
        let src = "fn f() -> Result<Vec<u8>, E> { Ok(fmt::Result) }";
        assert_eq!(unshadow_prelude(src, src, &scope, true).as_deref(), Some("fn f() -> ::core::result::Result<::std::vec::Vec<u8>, E> { Ok(fmt::Result) }"));
        assert_eq!(unshadow_prelude(src, src, &scope, false), None, "`Vec` needs std");
    }
}
