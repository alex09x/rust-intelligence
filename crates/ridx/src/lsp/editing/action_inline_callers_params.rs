/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The parameters of a function `Inline into all callers` rewrites: how each binds its argument, and which types a
//! `let` can write.

use super::action_generics_helpers::clean_param_ident;
use super::action_generics_sub::{split_top_level_colon, split_top_level_commas};
use super::action_parse_decl::collapse_ws;
use super::action_parse_items::{contains_word, skip_balanced};

#[derive(Clone)]
pub(crate) struct FnParamInfo {
    pub is_self: bool,
    pub self_kind: Option<SelfKind>,
    pub ident: Option<String>,
    pub is_mut_ref: bool,
    pub ty: Option<String>,
    /// The declared type when it names none of the item's generics, so a caller can write it on a `let`.
    pub decl_ty: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelfKind {
    Value,
    Ref,
    MutRef,
}

pub(crate) fn is_primitive_scalar(ty: &str) -> bool {
    matches!(ty.trim(), "u8" | "u16" | "u32" | "u64" | "u128" | "usize" | "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "f32" | "f64" | "bool" | "char" | "str")
}

pub(crate) fn parse_fn_params(params_src: &str, type_generics: &[String]) -> Option<Vec<FnParamInfo>> {
    let mut out = Vec::new();
    for raw_p in split_top_level_commas(params_src) {
        // A parameter's attributes (`#[rust_analyzer::rust_fixture]`) do not reach the call; a `cfg` may remove it.
        let mut t = raw_p.trim();
        while let Some(rest) = t.strip_prefix('#') {
            let rest = rest.trim_start();
            let end = skip_balanced(rest.as_bytes(), 0, b'[', b']')?;
            if contains_word(&rest[..end], "cfg") || contains_word(&rest[..end], "cfg_attr") {
                return None;
            }
            t = rest[end..].trim_start();
        }
        if t.is_empty() {
            continue;
        }
        if t == "self" || t == "mut self" {
            out.push(FnParamInfo { is_self: true, self_kind: Some(SelfKind::Value), ident: Some("this".to_string()), is_mut_ref: false, ty: None, decl_ty: None });
            continue;
        }
        if let Some(rest) = t.strip_prefix('&') {
            let rest_trimmed = rest.trim();
            let without_lifetime = if rest_trimmed.starts_with('\'') {
                if let Some(idx) = rest_trimmed.find(|c: char| c.is_ascii_whitespace()) {
                    rest_trimmed[idx..].trim()
                } else {
                    rest_trimmed
                }
            } else {
                rest_trimmed
            };
            if without_lifetime == "self" {
                out.push(FnParamInfo { is_self: true, self_kind: Some(SelfKind::Ref), ident: Some("this".to_string()), is_mut_ref: false, ty: None, decl_ty: None });
                continue;
            }
            if without_lifetime == "mut self" {
                out.push(FnParamInfo { is_self: true, self_kind: Some(SelfKind::MutRef), ident: Some("this".to_string()), is_mut_ref: true, ty: None, decl_ty: None });
                continue;
            }
        }
        let (lhs, Some(rhs)) = split_top_level_colon(t) else { return None };
        let l_trim = lhs.trim();
        let (is_mut, raw_id) = if let Some(r) = l_trim.strip_prefix("mut ") { (true, r.trim()) } else { (false, l_trim) };
        // `_: T` takes an argument it never names: it is still evaluated, by `let _: T = arg;`.
        let pid = if raw_id == "_" && !is_mut { "_".to_string() } else { clean_param_ident(raw_id)? };
        let pty = collapse_ws(rhs);
        // An inferred `_` type cannot be written on a `let`; the anonymous lifetime `'_` can.
        if pty.is_empty() || has_inferred_placeholder(&pty) {
            return None;
        }
        if pty.contains('[') && type_generics.iter().any(|g| contains_word(&pty, g)) {
            return None;
        }
        let is_mut_ref = pty.trim().starts_with("&mut ");
        let decl_ty = (!type_generics.iter().any(|g| contains_word(&pty, g))).then(|| pty.clone());
        let ty = if is_primitive_scalar(&pty) { Some(pty) } else { None };
        let bind_lhs = if is_mut { format!("mut {pid}") } else { pid };
        out.push(FnParamInfo { is_self: false, self_kind: None, ident: Some(bind_lhs), is_mut_ref, ty, decl_ty });
    }
    Some(out)
}

/// Whether a parameter type can be written on a `let`: no named lifetime (the anonymous `'_` is fine), `Self`,
/// `impl`/`dyn` or inferred `_` left.
pub(crate) fn is_writable_binding_type(norm_ty: &str) -> bool {
    let t = norm_ty.replace("'_", "");
    !t.contains('\'') && !contains_word(&t, "Self") && !t.contains("impl ") && !t.contains("dyn ") && !has_inferred_placeholder(&t)
}

/// Whether a type holds an inferred `_` (a standalone underscore, not part of a name like `regex_automata`).
pub(crate) fn has_inferred_placeholder(ty: &str) -> bool {
    let b = ty.as_bytes();
    ty.match_indices('_').any(|(i, _)| {
        let word = |c: &u8| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'\'';
        (i == 0 || !word(&b[i - 1])) && b.get(i + 1).is_none_or(|c| !word(c))
    })
}

/// A parameter type with its named reference lifetimes dropped (`&'a mut T` → `&mut T`).
pub(crate) fn normalize_param_type_for_binding(ty: &str) -> String {
    let mut out = String::with_capacity(ty.len());
    let bytes = ty.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&' && i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
            let mut j = i + 2;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                j += 1;
            }
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j + 3 < bytes.len() && &bytes[j..j + 4] == b"mut " {
                out.push_str("&mut ");
                i = j + 4;
            } else {
                out.push('&');
                i = j;
            }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

/// Whether the call `recv.name(args)` / `path::name(args)` (masked) reads only places: its receiver and every
/// argument are a variable, a field path or a reference to one, with no call, index or literal to make a temporary.
pub(crate) fn call_takes_only_places(call: &str) -> bool {
    // With a place receiver and a path, the first parenthesis opens the arguments; one inside a receiver (`f().m()`)
    // leaves arguments that hold another and are refused below.
    let Some(open) = call.find('(').filter(|_| call.trim_end().ends_with(')')) else { return false };
    let is_place = |e: &str| {
        let e = e.trim();
        let e = e.strip_prefix("&mut ").or_else(|| e.strip_prefix('&')).unwrap_or(e).trim();
        !e.is_empty()
            && e.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && e.split('.').all(|seg| !seg.is_empty() && seg.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_'))
    };
    let head = &call[..open];
    // A receiver is what precedes the method name's dot; a path call has none.
    if let Some(dot) = head.rfind('.') {
        if !is_place(&head[..dot]) {
            return false;
        }
    }
    let args = &call[open + 1..call.trim_end().len() - 1];
    !args.contains('(') && split_top_level_commas(args).iter().filter(|a| !a.trim().is_empty()).all(|a| is_place(a))
}
