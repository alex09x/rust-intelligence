/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Methods of impls for a particular shape of a generic type: `impl<T> Option<Option<T>> { fn flatten }` or
//! `impl<T> Option<&T> { fn copied }` apply only to receivers of that shape, as rust-analyzer checks.

/// Whether a method whose path names its impl's self type with arguments (`core::option::Option::<&T>::copied`)
/// applies to a receiver of type `recv_ty`. Paths without such arguments, and receivers whose arguments are not
/// known, always apply.
pub fn impl_shape_matches(def_path: &str, recv_ty: &str) -> bool {
    let Some((parent, _)) = def_path.rsplit_once("::") else { return true };
    let Some(open) = parent.find("::<") else { return true };
    let Some(impl_args) = top_level_args(&parent[open + 2..]) else { return true };
    let Some(recv_open) = recv_ty.find('<') else { return true };
    let Some(recv_args) = top_level_args(&recv_ty[recv_open..]) else { return true };
    impl_args.iter().zip(recv_args.iter()).all(|(i, r)| arg_matches(i, r))
}

fn arg_matches(impl_arg: &str, recv_arg: &str) -> bool {
    let (i, r) = (impl_arg.trim(), recv_arg.trim());
    if is_type_param(i) || r.is_empty() || r == "_" || r.starts_with('\'') {
        return true;
    }
    if let Some(rest) = i.strip_prefix('&') {
        let Some(r_rest) = r.strip_prefix('&') else { return false };
        let i_mut = rest.trim_start_matches(|c: char| c == '\'' || c.is_alphanumeric() || c == '_').trim_start().starts_with("mut ");
        let r_mut = r_rest.trim_start_matches(|c: char| c == '\'' || c.is_alphanumeric() || c == '_').trim_start().starts_with("mut ");
        return !i_mut || r_mut;
    }
    if r.starts_with('&') || r.starts_with('[') || r.starts_with('(') {
        return false;
    }
    head(i) == head(r)
}

/// A generic parameter name such as `T`, `E` or `Idx`: one path segment, no generics, starting uppercase.
fn is_type_param(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_ascii_uppercase()) && s.len() <= 3 && s.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// The last path segment of a type, without its generics: `core::option::Option<T>` → `Option`.
fn head(ty: &str) -> &str {
    let base = ty.split('<').next().unwrap_or(ty);
    base.rsplit("::").next().unwrap_or(base).trim()
}

/// The comma-separated arguments of the generic list starting at `text` (which begins with `<`).
fn top_level_args(text: &str) -> Option<Vec<&str>> {
    let body = text.strip_prefix('<')?;
    let mut depth = 0i32;
    let mut start = 0;
    let mut out = Vec::new();
    for (i, c) in body.char_indices() {
        match c {
            '<' | '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            '>' if depth == 0 => {
                out.push(&body[start..i]);
                return Some(out);
            }
            '>' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&body[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::impl_shape_matches;

    #[test]
    fn test_impl_shape_matches_receiver() {
        let copied = "core::option::Option::<&T>::copied";
        assert!(impl_shape_matches(copied, "Option<&str>"));
        assert!(!impl_shape_matches(copied, "Option<String>"));
        let flatten = "core::option::Option::<core::option::Option<T>>::flatten";
        assert!(impl_shape_matches(flatten, "core::option::Option<core::option::Option<u8>>"));
        assert!(!impl_shape_matches(flatten, "Option<u8>"));
        assert!(impl_shape_matches("core::option::Option::map", "Option<u8>"));
        assert!(impl_shape_matches(copied, "Option"));
    }
}
