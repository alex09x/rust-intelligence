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

/// Whether the body calls a conversion method on a parameter typed by a generic parameter or `impl Trait`
/// (`path.as_ref()` with `path: P`, `P: AsRef<Path>`): bound to the argument at the call, the bound that chose the
/// target type is gone and the call may become ambiguous (E0283).
pub(crate) fn generic_param_receives_call(params_src: &str, type_generics: &[String], body_masked: &str) -> bool {
    split_top_level_commas(params_src).iter().any(|p| {
        let (lhs, Some(ty)) = split_top_level_colon(p) else { return false };
        let id = lhs.trim().trim_start_matches("mut ").trim();
        let generic = contains_word(ty, "impl") || type_generics.iter().any(|g| contains_word(ty, g));
        generic
            && !id.is_empty()
            && body_masked.match_indices(id).any(|(i, _)| {
                let b = body_masked.as_bytes();
                let before_ok = i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_' || b[i - 1] == b'.');
                let after = &body_masked[i + id.len()..];
                let ends = !after.starts_with(|c: char| c.is_alphanumeric() || c == '_');
                // Only a conversion trait's method has an output the bound alone fixed; others are found on the
                // argument's own type as before.
                let method = after.trim_start().strip_prefix('.').map(|m| m.trim_start().split(|c: char| !(c.is_alphanumeric() || c == '_')).next().unwrap_or(""));
                before_ok && ends && method.is_some_and(|m| matches!(m, "as_ref" | "as_mut" | "into" | "try_into" | "borrow" | "borrow_mut"))
            })
    })
}

/// Whether the masked body has a `?` that returns from the function itself: one inside a closure
/// (`|it| it.next()?`), a macro's tokens (`quote! { x? }`), an `async` block or a nested `fn` returns from that
/// instead and does not count.
pub(crate) fn has_own_question_mark(masked: &str) -> bool {
    let b = masked.as_bytes();
    let close_of = |i: usize| -> usize {
        let (open, close) = match b[i] {
            b'(' => (b'(', b')'),
            b'[' => (b'[', b']'),
            _ => (b'{', b'}'),
        };
        skip_balanced(b, i, open, close).unwrap_or(b.len())
    };
    // The end of a closure body that is no block: the first `,` `;` or unmatched closer at depth 0.
    let expr_end = |mut i: usize| -> usize {
        let mut depth = 0i32;
        while i < b.len() {
            match b[i] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' if depth == 0 => return i,
                b')' | b']' | b'}' => depth -= 1,
                b',' | b';' if depth == 0 => return i,
                _ => {}
            }
            i += 1;
        }
        i
    };
    // Byte positions only: the text may hold multi-byte characters, and a position past the end is the end.
    let skip_ws = |mut j: usize| {
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        j
    };
    let find_from = |j: usize, c: u8| b.get(j..).and_then(|s| s.iter().position(|&x| x == c)).map_or(b.len(), |p| j + p);
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let mut q = i;
        while q > 0 && b[q - 1].is_ascii_whitespace() {
            q -= 1;
        }
        let prev = &b[..q];
        if c == b'?' {
            return true;
        }
        // A macro call's token tree.
        if c == b'!' && i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_') {
            let j = skip_ws(i + 1);
            if matches!(b.get(j), Some(b'(' | b'[' | b'{')) {
                i = close_of(j);
                continue;
            }
        }
        // A closure: `|params|` (or `||`) where an expression starts, then its body.
        let opens_expr = prev.is_empty() || matches!(prev.last(), Some(b'(' | b',' | b'=' | b'{' | b';' | b':')) || prev.ends_with(b"move") || prev.ends_with(b"return");
        if c == b'|' && opens_expr {
            let params_end = if b.get(i + 1) == Some(&b'|') { i + 1 } else { find_from(i + 1, b'|') };
            let mut j = skip_ws(params_end + 1);
            if b.get(j..).is_some_and(|s| s.starts_with(b"->")) {
                j = find_from(j, b'{');
            }
            i = if b.get(j) == Some(&b'{') { close_of(j) } else { expr_end(j) };
            continue;
        }
        // An `async` block or a nested `fn` item: its own `?`.
        let nested_fn = b[i..].starts_with(b"fn ") && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_'));
        if (c == b'{' && prev.ends_with(b"async")) || (c == b'{' && prev.ends_with(b"move") && prev[..prev.len() - 4].trim_ascii_end().ends_with(b"async")) || nested_fn {
            let open = if c == b'{' { i } else { find_from(i, b'{') };
            i = if open < b.len() { close_of(open) } else { b.len() };
            continue;
        }
        i += 1;
    }
    false
}

/// The masked body with every macro call's token tree (`quote! { … }`, `format!(…)`) blanked, same length: what the
/// function itself names, apart from what its macros receive as tokens.
pub(crate) fn mask_macro_tokens(masked: &str) -> String {
    let mut out = masked.as_bytes().to_vec();
    let b = masked.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'!' && i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_') {
            let mut j = i + 1;
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if let Some(&open) = b.get(j).filter(|c| matches!(c, b'(' | b'[' | b'{')) {
                let close = match open {
                    b'(' => b')',
                    b'[' => b']',
                    _ => b'}',
                };
                let end = skip_balanced(b, j, open, close).unwrap_or(b.len());
                for c in &mut out[j + 1..end.saturating_sub(1).max(j + 1)] {
                    if c.is_ascii() {
                        *c = b' ';
                    }
                }
                i = end;
                continue;
            }
        }
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| masked.to_string())
}

/// `src` with each whole word of `from` (outside comments and strings, not after `.` or `::`) replaced by the
/// matching `to` (`E::write_u32` -> `NE::write_u32`).
pub(crate) fn substitute_words(src: &str, from: &[String], to: &[String]) -> String {
    let masked = super::action_parse::mask_comments_and_strings(src);
    let b = masked.as_bytes();
    let mut out = src.to_string();
    let mut hits: Vec<(usize, usize, &str)> = Vec::new();
    for (f, t) in from.iter().zip(to) {
        for (i, _) in masked.match_indices(f.as_str()) {
            let e = i + f.len();
            let word = (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_')) && b.get(e).is_none_or(|c| !(c.is_ascii_alphanumeric() || *c == b'_'));
            let before = masked[..i].trim_end();
            if word && !before.ends_with('.') && !before.ends_with("::") && !before.ends_with('\'') {
                hits.push((i, e, t.as_str()));
            }
        }
    }
    hits.sort_by_key(|h| std::cmp::Reverse(h.0));
    for (i, e, t) in hits {
        out.replace_range(i..e, t);
    }
    out
}

/// `ty` with each named lifetime but `'static` written `'_` (`Walker<I<'db>>` → `Walker<I<'_>>`), for an annotation in
/// the caller's body, where the callee's lifetime names mean nothing.
pub(crate) fn anonymize_lifetimes(ty: &str) -> String {
    let mut out = String::with_capacity(ty.len());
    let mut rest = ty;
    while let Some(p) = rest.find('\'') {
        out.push_str(&rest[..=p]);
        let after = &rest[p + 1..];
        let n = after.len() - after.trim_start_matches(|c: char| c.is_alphanumeric() || c == '_').len();
        out.push_str(if &after[..n] == "static" { "static" } else { "_" });
        rest = &after[n..];
    }
    out.push_str(rest);
    out
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

#[cfg(test)]
mod tests {
    use super::{anonymize_lifetimes, generic_param_receives_call, has_own_question_mark, mask_macro_tokens, substitute_words};

    #[test]
    fn macro_tokens_are_not_the_functions_names() {
        let m = mask_macro_tokens("let x = quote! { impl Self { fn f(self) {} } }; Self::g()");
        assert!(!m[..m.find(';').unwrap()].contains("Self") && m.ends_with("Self::g()"));
        assert_eq!(mask_macro_tokens("vec![a, b]").len(), "vec![a, b]".len());
    }

    #[test]
    fn generic_parameters_take_the_turbofish_arguments() {
        let (from, to) = (vec!["E".to_string()], vec!["wire::NE".to_string()]);
        assert_eq!(substitute_words("E::write_u32(x, \"E\"); self.E; Ex::y", &from, &to), "wire::NE::write_u32(x, \"E\"); self.E; Ex::y");
    }

    #[test]
    fn only_the_functions_own_question_marks_count() {
        assert!(has_own_question_mark("let x = f()?; x"));
        assert!(!has_own_question_mark("std::iter::successors(Some(a), |it| match it.next()? { _ => None })"));
        assert!(!has_own_question_mark("quote! { let x = seq.next()?; }"));
        assert!(!has_own_question_mark("let g = async move { h().await? }; fn inner() -> Option<u8> { Some(1)? .into() } 1"));
        assert!(has_own_question_mark("v.iter().map(|x| x + 1).sum::<u8>().checked_add(1)?"));
        // A closure at the very end, multi-byte text before it: no position past the end, no split character.
        assert!(!has_own_question_mark("let é = 1; f(|"));
        assert!(!has_own_question_mark("let s = \"ä\"; g(|x|"));
        assert!(!has_own_question_mark("f(|| ->"));
    }

    #[test]
    fn a_method_on_a_generic_parameter_loses_its_bound() {
        let gens = vec!["P".to_string()];
        assert!(generic_param_receives_call("&self, path: P, into: &mut Vec<usize>", &gens, "f(&Candidate::new(path.as_ref()), into)"));
        assert!(generic_param_receives_call("s: impl AsRef<str>", &gens, "s.as_ref().len()"));
        assert!(!generic_param_receives_call("path: P", &gens, "g(path); pathname.len()"), "only moved on");
        assert!(!generic_param_receives_call("n: usize", &gens, "n.pow(2)"));
        assert!(!generic_param_receives_call("e: P", &gens, "e.custom(1)"), "a method the argument's type has itself");
    }

    #[test]
    fn named_lifetimes_become_inferred_and_static_stays() {
        assert_eq!(anonymize_lifetimes("TypeWalker<DbInterner<'db>>"), "TypeWalker<DbInterner<'_>>");
        assert_eq!(anonymize_lifetimes("&'a mut Foo<'b, 'static>"), "&'_ mut Foo<'_, 'static>");
        assert_eq!(anonymize_lifetimes("u32"), "u32");
    }
}
