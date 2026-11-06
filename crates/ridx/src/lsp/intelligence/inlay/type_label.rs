/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Types as rust-analyzer prints them in inlay hints: items by their name alone (`Vec<String>`, not
//! `std::vec::Vec<std::string::String>`), and type arguments past 25 characters cut to `…`.

/// rust-analyzer's default `inlayHints.maxLength`.
pub const MAX_LENGTH: usize = 25;

/// `ty` (as the index records it) in rust-analyzer's inlay-hint form, or `None` for a type not worth a hint.
pub fn hint_type(ty: &str) -> Option<String> {
    let ty = ty.trim();
    if ty.is_empty() || ty == "_" || ty.contains("{unknown}") || ty.contains("{type error}") || ty.contains("{closure") || ty.contains("{coroutine") || ty.contains("{async") {
        return None;
    }
    Some(truncate(&short_paths(ty), MAX_LENGTH))
}

/// Every path in `ty` cut to its last segment, except an associated item after a qualified self type
/// (`<Self as Serializer>::SerializeMap` keeps `::SerializeMap`).
pub fn short_paths(ty: &str) -> String {
    let chars: Vec<char> = ty.chars().collect();
    let mut out = String::with_capacity(ty.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_alphabetic() || c == '_' {
            let start = i;
            let mut last_seg_start = i;
            let mut j = i;
            loop {
                while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                if j + 2 < chars.len() && chars[j] == ':' && chars[j + 1] == ':' && (chars[j + 2].is_alphabetic() || chars[j + 2] == '_') {
                    j += 2;
                    last_seg_start = j;
                } else {
                    break;
                }
            }
            let keep_from = if start > 0 && chars[start - 1] == ':' { start } else { last_seg_start };
            out.extend(&chars[keep_from..j]);
            i = j;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// `ty` with each type argument and tuple element that starts past `max` characters replaced by `…`, and the rest
/// of its list dropped, as rust-analyzer's truncated display does.
pub fn truncate(ty: &str, max: usize) -> String {
    let chars: Vec<char> = ty.chars().collect();
    let mut out = String::new();
    let mut out_len = 0usize;
    let mut i = 0;
    let mut depth = 0i32;
    let push = |out: &mut String, out_len: &mut usize, c: char| {
        out.push(c);
        *out_len += 1;
    };
    while i < chars.len() {
        let c = chars[i];
        let element_start = match c {
            '<' | '(' => true,
            ',' if depth > 0 => true,
            _ => false,
        };
        match c {
            '<' | '(' | '[' => depth += 1,
            '>' if i > 0 && chars[i - 1] == '-' => {}
            '>' | ')' | ']' => depth -= 1,
            _ => {}
        }
        push(&mut out, &mut out_len, c);
        i += 1;
        if element_start {
            while i < chars.len() && chars[i] == ' ' {
                push(&mut out, &mut out_len, ' ');
                i += 1;
            }
            let closes_now = i < chars.len() && matches!(chars[i], '>' | ')');
            if out_len >= max && !closes_now {
                push(&mut out, &mut out_len, '…');
                // Skip the rest of this list up to its closing bracket.
                let mut d = 0i32;
                while i < chars.len() {
                    match chars[i] {
                        '<' | '(' | '[' => d += 1,
                        '>' if chars[i - 1] == '-' => {}
                        '>' | ')' | ']' if d == 0 => break,
                        '>' | ')' | ']' => d -= 1,
                        _ => {}
                    }
                    i += 1;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hint_types_like_rust_analyzer() {
        assert_eq!(hint_type("std::vec::Vec<std::string::String>").as_deref(), Some("Vec<String>"));
        assert_eq!(hint_type("<Self as ser::Serializer>::SerializeMap").as_deref(), Some("<Self as Serializer>::SerializeMap"));
        assert_eq!(hint_type("syn::punctuated::Punctuated<syn::GenericParam, syn::token::Comma>").as_deref(), Some("Punctuated<GenericParam, …>"));
        assert_eq!(hint_type("impl std::iter::Iterator<Item = (usize, &str)>").as_deref(), Some("impl Iterator<Item = (usize, …)>"));
        assert_eq!(hint_type("&'static str").as_deref(), Some("&'static str"));
        assert_eq!(hint_type("{closure}"), None);
        assert_eq!(hint_type("fn(&str) -> bool").as_deref(), Some("fn(&str) -> bool"));
    }
}
