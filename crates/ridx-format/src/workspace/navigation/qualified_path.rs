/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Spellings of a qualified item path (`<Type as Trait>::name`) in other crates.
//!
//! The crate that defines an item writes the paths of its own types and traits relative to its
//! root (`<stats::Stats as core::clone::Clone>::clone`); a crate that uses the item writes them
//! from its own root, with the defining crate in front (`<grep_printer::stats::Stats as
//! core::clone::Clone>::clone`). References are indexed by the user's spelling, so a lookup by
//! the defining crate's spelling alone misses every call from another crate.

/// The index of the `>` that closes the `<` the string starts with, and of the ` as ` between the
/// type and the trait, both at the nesting depth of that first `<`.
fn split_qualified(path: &str) -> Option<(usize, usize)> {
    if !path.starts_with('<') {
        return None;
    }
    let (mut depth, mut sep) = (0usize, None);
    let bytes = path.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'<' => depth += 1,
            b'>' if i > 0 && bytes[i - 1] == b'-' => {}
            b'>' => {
                depth -= 1;
                if depth == 0 {
                    return sep.map(|s| (s, i));
                }
            }
            b' ' if depth == 1 && sep.is_none() && path[i..].starts_with(" as ") => sep = Some(i),
            _ => {}
        }
    }
    None
}

/// A component that a crate can prefix: a path, not `&T`, `[T]`, `(A, B)` or `dyn`.
fn is_path(component: &str) -> bool {
    component
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && !component.starts_with("dyn ")
}

/// The spellings of the qualified `path` as a crate named `krate` that depends on the defining
/// crate writes it: the type, the trait, or both prefixed with `krate::`. Empty for a path that
/// is not of the form `<Type as Trait>::name`.
pub(super) fn consumer_spellings(path: &str, krate: &str) -> Vec<String> {
    let Some((sep, close)) = split_qualified(path) else {
        return Vec::new();
    };
    let (ty, tr, tail) = (&path[1..sep], &path[sep + 4..close], &path[close..]);
    let prefixed = |c: &str| {
        if is_path(c) {
            format!("{krate}::{c}")
        } else {
            c.to_string()
        }
    };
    let mut out = Vec::new();
    for (t, r) in [
        (prefixed(ty), tr.to_string()),
        (ty.to_string(), prefixed(tr)),
        (prefixed(ty), prefixed(tr)),
    ] {
        let spelled = format!("<{t} as {r}{tail}");
        if spelled != path && !out.contains(&spelled) {
            out.push(spelled);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::consumer_spellings;

    #[test]
    fn the_type_the_trait_or_both_get_the_crate_in_front() {
        assert_eq!(
            consumer_spellings("<stats::Stats as sink::Sink>::clone", "grep_printer"),
            vec![
                "<grep_printer::stats::Stats as sink::Sink>::clone",
                "<stats::Stats as grep_printer::sink::Sink>::clone",
                "<grep_printer::stats::Stats as grep_printer::sink::Sink>::clone",
            ]
        );
    }

    #[test]
    fn nested_generics_and_arrows_do_not_end_the_type_early() {
        let v = consumer_spellings(
            "<Take<St> as futures_core::FusedStream>::is_terminated",
            "tokio_stream",
        );
        assert_eq!(
            v[0],
            "<tokio_stream::Take<St> as futures_core::FusedStream>::is_terminated"
        );
        let f = consumer_spellings("<Box<dyn Fn() -> u8> as Tr>::call", "k");
        assert_eq!(f[0], "<k::Box<dyn Fn() -> u8> as Tr>::call");
    }

    #[test]
    fn a_reference_or_slice_type_is_not_prefixed_and_other_paths_have_no_spelling() {
        let v = consumer_spellings("<&'a str as Tr>::f", "k");
        assert_eq!(v, vec!["<&'a str as k::Tr>::f"]);
        assert!(consumer_spellings("stats::Stats::clone", "k").is_empty());
        assert!(consumer_spellings("sys::<impl Tr for S>::f", "k").is_empty());
        assert!(consumer_spellings("<S as Tr", "k").is_empty());
    }
}
