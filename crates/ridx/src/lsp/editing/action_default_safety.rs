/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Identifier resolution and shadowing guards for cross-file trait default methods.

use super::action_generics_sub::split_top_level_commas;
use super::action_parse::is_ident_byte;
use super::action_parse_decl::next_ident_token;
use std::collections::HashSet;

#[allow(clippy::too_many_arguments)]
pub fn is_default_fn_compile_safe(
    fn_masked: &str,
    params_masked: &str,
    fn_gen_names: &HashSet<String>,
    same_module: bool,
    same_crate: bool,
    trait_fn_names: &HashSet<String>,
    doc_scope: &HashSet<String>,
    impl_hdr_idents: &HashSet<String>,
    std_method: &dyn Fn(&str) -> bool,
) -> bool {
    // An `unsafe` block is copied as it is: the impl runs the same code the default would, under the same contract.
    if fn_masked.contains('$') {
        return false;
    }
    let locals = collect_local_bindings(fn_masked, params_masked);
    let b = fn_masked.as_bytes();
    let mut prev_ident = "";
    let mut i = 0usize;
    while i < b.len() {
        if b[i] == b'\'' {
            i += 1;
            while i < b.len() && is_ident_byte(b[i]) {
                i += 1;
            }
            continue;
        }
        if b[i].is_ascii_alphabetic() || b[i] == b'_' {
            let Some((raw_tok, end_tok)) = next_ident_token(fn_masked, i) else {
                i += 1;
                continue;
            };
            let tok = raw_tok.strip_prefix("r#").unwrap_or(raw_tok);
            let prev_non_ws = fn_masked[..i].trim_end().as_bytes().last().copied().unwrap_or(0);
            let prev_is_path = i >= 2 && &b[i - 2..i] == b"::";
            let after_trim = fn_masked[end_tok..].trim_start();
            let next_is_path = after_trim.starts_with("::");
            let next_is_single_colon = after_trim.starts_with(':') && !next_is_path;

            if prev_ident == "fn" {
                prev_ident = tok;
                i = end_tok;
                continue;
            }
            prev_ident = tok;

            if !same_module && !prev_is_path && (tok == "super" || (tok == "self" && next_is_path)) {
                return false;
            }
            if !same_crate && !prev_is_path && tok == "crate" {
                return false;
            }
            if prev_non_ws == b'.' {
                if !same_module && after_trim.starts_with('(') && !trait_fn_names.contains(tok) && !is_safe_std_method(tok) && !std_method(tok) {
                    return false;
                }
                // A field of another crate's type is reachable only when the index found it public.
                if !same_crate && !after_trim.starts_with('(') && !after_trim.starts_with("::") && tok != "await" && !std_method(tok) {
                    return false;
                }
                i = end_tok;
                continue;
            }
            if !prev_is_path && next_is_path && locals.contains(tok) && !matches!(tok, "self" | "Self" | "super" | "crate") {
                return false;
            }
            if !same_module && !prev_is_path && !next_is_single_colon {
                let allowed = is_rust_kw_or_prelude(tok) || fn_gen_names.contains(tok) || impl_hdr_idents.contains(tok) || locals.contains(tok) || doc_scope.contains(tok);
                if !allowed {
                    return false;
                }
            }
            i = end_tok;
            continue;
        }
        i += 1;
    }
    true
}

pub(crate) fn collect_local_bindings(fn_masked: &str, params_masked: &str) -> HashSet<String> {
    let mut locals = HashSet::new();
    locals.insert("self".to_string());
    for p in split_top_level_commas(params_masked) {
        let lhs = p.split(':').next().unwrap_or("").trim();
        add_lowercase_idents(lhs, &mut locals);
    }
    let b = fn_masked.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        if b[i] == b'|' && b.get(i + 1).copied() != Some(b'|') && (i == 0 || b[i - 1] != b'|') {
            if let Some(close_bar) = fn_masked[i + 1..].find('|') {
                add_lowercase_idents(&fn_masked[i + 1..i + 1 + close_bar], &mut locals);
                i += close_bar + 2;
                continue;
            }
        }
        if b[i].is_ascii_alphabetic() || b[i] == b'_' {
            if let Some((tok, end_tok)) = next_ident_token(fn_masked, i) {
                if tok == "let" {
                    let rest = &fn_masked[end_tok..];
                    let stop = rest.find(['=', ';', ':']).unwrap_or(rest.len());
                    add_lowercase_idents(&rest[..stop], &mut locals);
                } else if tok == "for" {
                    let rest = &fn_masked[end_tok..];
                    if let Some(in_pos) = rest.find(" in ") {
                        add_lowercase_idents(&rest[..in_pos], &mut locals);
                    }
                }
                i = end_tok;
                continue;
            }
        }
        if i + 2 <= b.len() && &b[i..i + 2] == b"=>" {
            let line_back = fn_masked[..i].rfind(['{', ',', ';']).map(|p| p + 1).unwrap_or(0);
            add_pattern_destructure_idents(&fn_masked[line_back..i], &mut locals);
            i += 2;
            continue;
        }
        i += 1;
    }
    locals
}

fn add_pattern_destructure_idents(pat: &str, locals: &mut HashSet<String>) {
    let b = pat.as_bytes();
    let mut depth = 0i32;
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b'(' | b'{' | b'[' => {
                depth += 1;
                i += 1;
            }
            b')' | b'}' | b']' => {
                depth = (depth - 1).max(0);
                i += 1;
            }
            _ if depth > 0 && (b[i].is_ascii_alphabetic() || b[i] == b'_') => {
                if let Some((tok, end_tok)) = next_ident_token(pat, i) {
                    let clean = tok.strip_prefix("r#").unwrap_or(tok);
                    if clean.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c == '_') && !is_rust_kw_or_prelude(clean) {
                        locals.insert(clean.to_string());
                    }
                    i = end_tok;
                    continue;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
}

fn add_lowercase_idents(s: &str, locals: &mut HashSet<String>) {
    let mut i = 0usize;
    let b = s.as_bytes();
    while i < b.len() {
        if b[i].is_ascii_alphabetic() || b[i] == b'_' {
            if let Some((tok, end_tok)) = next_ident_token(s, i) {
                let clean = tok.strip_prefix("r#").unwrap_or(tok);
                if clean.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c == '_') && !is_rust_kw_or_prelude(clean) {
                    locals.insert(clean.to_string());
                }
                i = end_tok;
                continue;
            }
        }
        i += 1;
    }
}

#[rustfmt::skip]
const RUST_KW_AND_PRELUDE: &[&str] = &[
    "_", "AsMut", "AsRef", "Box", "Clone", "Copy", "Default", "DoubleEndedIterator", "Drop", "Eq", "Err",
    "ExactSizeIterator", "Extend", "Fn", "FnMut", "FnOnce", "From", "Hash", "Into", "IntoIterator", "Iterator",
    "None", "Ok", "Option", "Ord", "PartialEq", "PartialOrd", "Result", "Self", "Send", "Sized", "Some", "String",
    "Sync", "ToOwned", "ToString", "TryFrom", "TryInto", "Unpin", "Vec", "alloc", "as", "assert", "assert_eq",
    "assert_ne", "async", "await", "bool", "break", "cfg", "char", "concat", "const", "continue", "core", "crate",
    "debug_assert", "debug_assert_eq", "debug_assert_ne", "dyn", "else", "enum", "env", "eprint", "eprintln", "extern",
    "f32", "f64", "false", "fn", "for", "format", "format_args", "i128", "i16", "i32", "i64", "i8", "if", "impl",
    "in", "include_str", "isize", "let", "loop", "match", "matches", "mod", "move", "mut", "option_env", "panic",
    "print", "println", "pub", "ref", "return", "self", "static", "std", "str", "struct", "super", "todo", "trait",
    "true", "type", "u128", "u16", "u32", "u64", "u8", "unimplemented", "unreachable", "unsafe", "use", "usize",
    "vec", "where", "while", "write", "writeln",
];

#[rustfmt::skip]
const SAFE_STD_METHODS: &[&str] = &[
    "all", "and_then", "any", "as_bytes", "as_deref", "as_deref_mut", "as_mut", "as_ref", "as_slice", "as_str",
    "by_ref", "bytes", "chain", "chars", "checked_add", "checked_sub", "clear", "clone", "clone_from", "cloned",
    "cmp", "collect", "contains", "copied", "ends_with", "enumerate", "eq", "err", "expect", "extend", "filter",
    "filter_map", "find", "flat_map", "fmt", "fuse", "into", "into_iter", "is_empty", "is_err", "is_none", "is_ok",
    "is_some", "iter", "iter_mut", "len", "map", "map_err", "map_or", "map_or_else", "max",
    "min", "ne", "ok", "or_else", "partial_cmp", "peekable", "push", "push_str", "rev", "saturating_add",
    "saturating_sub", "size_hint", "skip", "split", "starts_with", "take", "to_owned", "to_string", "trim",
    "unwrap", "unwrap_or", "unwrap_or_default", "unwrap_or_else", "wrapping_add", "write_fmt", "write_str", "zip",
];

pub(crate) fn is_rust_kw_or_prelude(tok: &str) -> bool {
    RUST_KW_AND_PRELUDE.contains(&tok)
}

fn is_safe_std_method(m: &str) -> bool {
    SAFE_STD_METHODS.contains(&m)
}
