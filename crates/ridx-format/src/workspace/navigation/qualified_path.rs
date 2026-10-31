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

/// Splits `<Type as Trait>::tail` into `(Type, Trait, ::tail)`.
pub(super) fn split_qualified_parts(path: &str) -> Option<(&str, &str, &str)> {
    let (sep, close) = split_qualified(path)?;
    Some((&path[1..sep], &path[sep + 4..close], &path[close + 1..]))
}

/// Extracts the implemented trait from either `<Type as Trait>::tail` or `[module::]<impl Trait for Type>::tail`.
pub(super) fn extract_impl_trait(path: &str) -> Option<&str> {
    if let Some((_, tr, _)) = split_qualified_parts(path) {
        return Some(tr);
    }
    let idx = path.find("<impl ")?;
    if idx > 0 && !path[..idx].ends_with("::") {
        return None;
    }
    let rest = &path[idx..];
    let bytes = rest.as_bytes();
    let (mut depth, mut for_pos, mut close) = (0usize, None, None);
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'<' => depth += 1,
            b'>' if i > 0 && bytes[i - 1] == b'-' => {}
            b'>' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(i);
                    break;
                }
            }
            b' ' if depth == 1 && for_pos.is_none() && rest[i..].starts_with(" for ") => for_pos = Some(i),
            _ => {}
        }
    }
    let (fp, _cl) = (for_pos?, close?);
    Some(rest[6..fp].trim())
}

fn first_ident_segment(s: &str) -> Option<&str> {
    let mut t = s.trim_start();
    loop {
        if let Some(rest) = t.strip_prefix('&') {
            t = rest.trim_start();
            if let Some(after_lt) = t.strip_prefix('\'') {
                t = after_lt.trim_start_matches(|c: char| c.is_ascii_alphanumeric() || c == '_').trim_start();
            }
            if let Some(after_mut) = t.strip_prefix("mut ") {
                t = after_mut.trim_start();
            }
            continue;
        }
        if let Some(rest) = t.strip_prefix(['(', '[', '*']) {
            t = rest.trim_start();
            continue;
        }
        if let Some(rest) = t.strip_prefix("mut ").or_else(|| t.strip_prefix("const ")).or_else(|| t.strip_prefix("dyn ")) {
            t = rest.trim_start();
            continue;
        }
        break;
    }
    let (seg, _) = t.split_once("::")?;
    let seg = seg.split(['<', '>', ' ', ')', ']']).next().unwrap_or(seg);
    (!seg.is_empty() && seg.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')).then_some(seg)
}

fn crate_tier(c: &str) -> u8 {
    match c {
        "core" => 1,
        "alloc" => 2,
        "std" | "proc_macro" => 3,
        _ => 4,
    }
}

/// Extracts the root crate name that owns `path`, handling `<Type as Trait>::item` via Rust's
/// coherence hierarchy (`core < alloc < std < user_crate`).
pub(crate) fn extract_root_crate(path: &str) -> &str {
    if let Some((ty, tr, _)) = split_qualified_parts(path) {
        let c_ty = first_ident_segment(ty);
        let c_tr = first_ident_segment(tr);
        match (c_ty, c_tr) {
            (Some(ct), Some(cr)) => {
                if crate_tier(cr) > crate_tier(ct) {
                    return cr;
                }
                return ct;
            }
            (Some(ct), None) => return ct,
            (None, Some(cr)) => return cr,
            (None, None) => {}
        }
    }
    if let Some(inner) = path.strip_prefix('<') {
        if let Some(c) = first_ident_segment(inner) {
            return c;
        }
    }
    let seg = path.split("::").next().unwrap_or(path);
    seg.split(['<', '>', ' ']).next().unwrap_or(seg)
}

/// A component that a crate can prefix: a path, not `&T`, `[T]`, `(A, B)` or `dyn`.
fn is_path(component: &str) -> bool {
    component.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && !component.starts_with("dyn ")
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
    for (t, r) in [(prefixed(ty), tr.to_string()), (ty.to_string(), prefixed(tr)), (prefixed(ty), prefixed(tr))] {
        let spelled = format!("<{t} as {r}{tail}");
        if spelled != path && !out.contains(&spelled) {
            out.push(spelled);
        }
    }
    if let Some((outer, inner_rest)) = ty.split_once('<') {
        if let Some(inner) = inner_rest.strip_suffix('>') {
            if is_path(inner) && !inner.starts_with(&format!("{krate}::")) {
                let ty_inner = format!("{outer}<{krate}::{inner}>");
                for r in [tr.to_string(), prefixed(tr)] {
                    let spelled = format!("<{ty_inner} as {r}{tail}");
                    if spelled != path && !out.contains(&spelled) {
                        out.push(spelled);
                    }
                }
            }
        }
    }
    out
}

/// Consumer crate spellings for `[module::]<impl Type>::tail` and `[module::]<impl Trait for Type>::tail`.
pub(super) fn impl_consumer_spellings(path: &str, krate: &str) -> Vec<String> {
    let Some(idx) = path.find("<impl ") else {
        return Vec::new();
    };
    if idx > 0 && !path[..idx].ends_with("::") {
        return Vec::new();
    }
    let mod_prefix = &path[..idx];
    let rest = &path[idx..];
    let bytes = rest.as_bytes();
    let (mut depth, mut for_pos, mut close) = (0usize, None, None);
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'<' => depth += 1,
            b'>' if i > 0 && bytes[i - 1] == b'-' => {}
            b'>' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(i);
                    break;
                }
            }
            b' ' if depth == 1 && for_pos.is_none() && rest[i..].starts_with(" for ") => for_pos = Some(i),
            _ => {}
        }
    }
    let Some(cl) = close else {
        return Vec::new();
    };
    let tail = &rest[cl..];
    let pref_mod = if mod_prefix.is_empty() || mod_prefix.starts_with(&format!("{krate}::")) { mod_prefix.to_string() } else { format!("{krate}::{mod_prefix}") };
    let pref_comp = |c: &str| {
        if is_path(c) && !c.starts_with(&format!("{krate}::")) && !matches!(extract_root_crate(c), "core" | "alloc" | "std") {
            format!("{krate}::{c}")
        } else {
            c.to_string()
        }
    };
    let mut out = Vec::new();
    if let Some(fp) = for_pos {
        let (tr, ty) = (rest[6..fp].trim(), rest[fp + 5..cl].trim());
        for (r, t) in [(pref_comp(tr), ty.to_string()), (tr.to_string(), pref_comp(ty)), (pref_comp(tr), pref_comp(ty))] {
            let spelled = format!("{pref_mod}<impl {r} for {t}{tail}");
            if spelled != path && !out.contains(&spelled) {
                out.push(spelled);
            }
        }
    } else {
        let ty = rest[6..cl].trim();
        let spelled = format!("{pref_mod}<impl {}{tail}", pref_comp(ty));
        if spelled != path {
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
        let v = consumer_spellings("<Take<St> as futures_core::FusedStream>::is_terminated", "tokio_stream");
        assert_eq!(v[0], "<tokio_stream::Take<St> as futures_core::FusedStream>::is_terminated");
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

    #[test]
    fn extract_root_crate_respects_coherence_hierarchy() {
        use super::extract_root_crate;
        assert_eq!(extract_root_crate("<T as alloc::string::ToString>::to_string"), "alloc");
        assert_eq!(extract_root_crate("<alloc::string::String as core::ops::Deref>::deref"), "alloc");
        assert_eq!(extract_root_crate("<&[u8] as std::io::Write>::write_all"), "std");
        assert_eq!(extract_root_crate("<my_crate::stats::Stats as core::clone::Clone>::clone"), "my_crate");
        assert_eq!(extract_root_crate("<core::pin::Pin<&mut T> as async_rt::io::AsyncRead>::poll_read"), "async_rt");
        assert_eq!(extract_root_crate("core::task::poll::Poll::Ready"), "core");
    }

    #[test]
    fn extract_impl_trait_and_impl_consumer_spellings() {
        use super::{extract_impl_trait, impl_consumer_spellings};
        assert_eq!(extract_impl_trait("fmt_mod::<impl fmt_mod::Encoder for &mut core::fmt::Formatter<'a>>::EncodeMap"), Some("fmt_mod::Encoder"));
        assert_eq!(extract_impl_trait("ast::expr_ext::<impl ast::generated::nodes::BinExpr>::rhs"), None);
        assert_eq!(
            impl_consumer_spellings("ast::expr_ext::<impl ast::generated::nodes::BinExpr>::rhs", "syntax"),
            vec!["syntax::ast::expr_ext::<impl syntax::ast::generated::nodes::BinExpr>::rhs"]
        );
    }
}
