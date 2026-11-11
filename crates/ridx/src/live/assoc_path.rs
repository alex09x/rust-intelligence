/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Module extraction from the index path of an associated item.

/// The module part of an associated item path `m::n::Type::<'a, T>::item`: the item, the owner's generic segment and the owner name removed.
pub(super) fn assoc_owner_module(path: &str) -> &str {
    let mut depth = 0i32;
    let mut cuts = [0usize; 3];
    let mut n = 0usize;
    let b = path.as_bytes();
    let mut i = b.len();
    while i >= 2 && n < 3 {
        i -= 1;
        match b[i] {
            b'>' if i == 0 || b[i - 1] != b'-' => depth += 1,
            b'<' => depth -= 1,
            b':' if depth == 0 && b[i - 1] == b':' => {
                i -= 1;
                cuts[n] = i;
                n += 1;
            }
            _ => {}
        }
    }
    if n == 0 {
        return "";
    }
    let has_generic_seg = n >= 2 && b.get(cuts[1] + 2) == Some(&b'<');
    let k = if has_generic_seg { 2 } else { 1 };
    if n > k {
        &path[..cuts[k]]
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::assoc_owner_module;

    #[test]
    fn test_assoc_owner_module_skips_owner_and_generic_segment() {
        assert_eq!(assoc_owner_module("ast::parse::ParserI::<'s, P>::parse_group"), "ast::parse");
        assert_eq!(assoc_owner_module("ast::parse::Parser::new"), "ast::parse");
        assert_eq!(assoc_owner_module("m::Wrap::<fn(A) -> B, C<D>>::call"), "m");
        assert_eq!(assoc_owner_module("Top::<T>::get"), "");
        assert_eq!(assoc_owner_module("Top::get"), "");
        assert_eq!(assoc_owner_module("lonely"), "");
    }
}
