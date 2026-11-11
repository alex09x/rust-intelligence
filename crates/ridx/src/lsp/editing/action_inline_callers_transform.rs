/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Inlining body transformations, rewrite rules, and return unwrapping.

use super::action_generics_sub::split_top_level_commas;
use super::action_inline_callers_helpers::{
    find_path_start, format_inlined_call, is_writable_binding_type, normalize_param_type_for_binding, receiver_chain_start, FnParamInfo, SelfKind,
};
use super::action_inline_callers_params::substitute_words;
use super::action_parse::{is_ident_byte, mask_comments_and_strings};
use super::action_parse_items::{contains_word, skip_balanced};

#[allow(clippy::too_many_arguments)]
pub(crate) fn process_call_site(
    src: &str,
    masked: &str,
    starts: &[usize],
    ref_byte: usize,
    name: &str,
    self_kind: Option<SelfKind>,
    params: &[FnParamInfo],
    body_src: &str,
    file_opt: Option<&str>,
    ws_opt: Option<&ridx_format::WorkspaceIndex>,
    is_fully_pub: bool,
    borrowed_return: bool,
    fn_generics: &[String],
) -> Option<(usize, usize, String)> {
    let mb = masked.as_bytes();
    let name_end = ref_byte + name.len();
    if name_end > src.len() || &src[ref_byte..name_end] != name {
        return None;
    }

    let mut p = name_end;
    while p < src.len() && mb[p].is_ascii_whitespace() {
        p += 1;
    }
    let mut turbofish: Option<Vec<String>> = None;
    if p + 2 < src.len() && &src[p..p + 2] == "::" {
        let mut q = p + 2;
        while q < src.len() && mb[q].is_ascii_whitespace() {
            q += 1;
        }
        if q < src.len() && mb[q] == b'<' {
            if let Some(close) = skip_balanced(mb, q, b'<', b'>') {
                turbofish = Some(split_top_level_commas(&src[q + 1..close - 1]).into_iter().map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect());
                p = close;
                while p < src.len() && mb[p].is_ascii_whitespace() {
                    p += 1;
                }
            }
        }
    }
    if p >= src.len() || mb[p] != b'(' {
        return None;
    }
    // The function's own generic parameters the body names take the call's turbofish (`write_to::<NE>(…)`), each a
    // plain path; without one the body's `E` would mean nothing here.
    let substituted;
    let body_src = match &turbofish {
        _ if body_src.trim().is_empty() => body_src,
        Some(args) if args.len() == fn_generics.len() && args.iter().all(|a| a.split("::").all(|s| !s.is_empty() && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_'))) => {
            substituted = substitute_words(body_src, fn_generics, args);
            &substituted
        }
        Some(_) => return None,
        None if fn_generics.iter().any(|g| contains_word(&mask_comments_and_strings(body_src), g)) => return None,
        None => body_src,
    };
    let call_end = skip_balanced(mb, p, b'(', b')')?;
    let raw_args: Vec<String> = split_top_level_commas(&src[p + 1..call_end - 1]).into_iter().filter(|a| !a.trim().is_empty()).collect();

    let mut recv_in_match = false;
    let (call_start, recv_expr, recv_ty) = if self_kind.is_some() {
        let before_dot = src[..ref_byte].trim_end();
        if !before_dot.ends_with('.') || before_dot.ends_with("..") {
            return None;
        }
        let dot_byte = before_dot.len() - 1;
        // A receiver that is a call or an index (`Foo::new().m()`, `v[i].m()`) is bound once by `let this`, a chain
        // with a later call (`a.b().c().m()`) by a `match`.
        let (recv_start, chained) = receiver_chain_start(src, masked, dot_byte)?;
        // Any temporary of the receiver (`LocalSet::new().run_until(f).await`) lived as long as the call's statement;
        // a `match` keeps it so, where `let this = &…;` would end it with the block a returned `async` value outlives.
        recv_in_match = chained || masked[recv_start..dot_byte].contains('(');
        if recv_start > 0 && (mb[recv_start - 1] == b'.' || is_ident_byte(mb[recv_start - 1])) {
            return None;
        }
        let recv = src[recv_start..dot_byte].trim();
        // A `self` taken by value cannot be moved out of an index (`v[i]`).
        let moves_out_of_index = self_kind == Some(SelfKind::Value) && masked[recv_start..dot_byte].contains('[');
        if recv.is_empty() || recv.starts_with('.') || recv.ends_with('.') || moves_out_of_index {
            return None;
        }
        let rty = match (file_opt, ws_opt) {
            (Some(f), Some(w)) => {
                let end_byte = src[..dot_byte].trim_end().len().saturating_sub(1);
                let l0 = starts.partition_point(|&s| s <= end_byte).saturating_sub(1);
                let l1 = (l0 + 1) as u32;
                let c1 = (1 + src[starts[l0]..end_byte].chars().count()) as u32;
                w.type_at(f, l1, c1)
            }
            _ => None,
        };
        (recv_start, Some(recv), rty)
    } else {
        let path_start = find_path_start(masked, ref_byte);
        (path_start, None, None)
    };

    // A borrowed result reaches the caller only through a receiver whose type says how to reborrow it.
    if borrowed_return && recv_expr.is_some() && recv_ty.is_none() {
        return None;
    }
    let mut recv_ty = recv_ty;
    if is_fully_pub {
        if self_kind == Some(SelfKind::MutRef) {
            let recv_str = recv_expr.unwrap_or("");
            let is_mut_ref = recv_ty.is_some_and(|t| t.starts_with("&mut "));
            let is_explicit_mut = recv_str.starts_with("&mut ") || recv_str == "self" || recv_str == "this";
            if !is_mut_ref && !is_explicit_mut {
                // Without a type the receiver's declaration decides; a body dereferencing `self` needs the exact one.
                match super::action_inline_callers_recv::mut_receiver_binding(masked, call_start, recv_str.trim()) {
                    Some(super::action_inline_callers_recv::MutRecv::Reborrow) => recv_ty = Some("&mut _"),
                    Some(super::action_inline_callers_recv::MutRecv::Borrow)
                        if recv_ty.is_none_or(|t| !t.starts_with('&')) && !body_src.contains("*self") && !body_src.contains("*this") => {}
                    _ => return None,
                }
            }
        } else if self_kind == Some(SelfKind::Value) && recv_ty.is_none() {
            return None;
        }
    }

    let non_self_count = params.iter().filter(|p| !p.is_self).count();
    if raw_args.len() != non_self_count {
        return None;
    }
    // Every argument is bound by a `let` before the body, in order, so a call argument evaluates once and in place.
    // Refused: a closure (its `|a, b|` would split on the comma) and an argument naming a parameter bound before it,
    // which would read that binding instead of the caller's variable.
    // Each argument masked on its own: a string literal masked in place is blank, and would be lost as an argument.
    let masked_args: Vec<String> = raw_args.iter().map(|a| mask_comments_and_strings(a)).collect();
    let plain: Vec<&FnParamInfo> = params.iter().filter(|p| !p.is_self).collect();
    for (i, arg) in masked_args.iter().enumerate() {
        let earlier_bound = plain.iter().take(i).filter_map(|p| p.ident.as_deref()).map(|id| id.trim_start_matches("mut ")).any(|id| contains_word(arg, id))
            || (self_kind.is_some() && contains_word(arg, "this"));
        if arg.contains('|') || earlier_bound {
            return None;
        }
        // An `impl Trait` parameter takes only a variable or a reference to one: a literal bound by `let` has no type
        // to infer (`let idx = 1;` then `idx.into_resettable()`), and an expression's type is unknown here.
        let impl_param = plain.get(i).is_some_and(|p| p.decl_ty.as_deref().or(p.ty.as_deref()).is_some_and(|t| contains_word(t, "impl")));
        let a = arg.trim();
        let a = a.strip_prefix("&mut ").or_else(|| a.strip_prefix('&')).unwrap_or(a).trim();
        let is_variable =
            a.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') && a.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') && !matches!(a, "true" | "false");
        if impl_param && !is_variable {
            return None;
        }
        // A call argument is bound with the parameter's type written out, else its generic return stays uninferred
        // (E0282); next to a receiver binding a `self` in it would borrow twice (E0502). For a reference parameter
        // `let x: &T = &f();` extends only the outermost temporary, so a chain through another call (`a.to_string()
        // .as_bytes()`) would drop the inner one at the `;` (E0716).
        // Without a written type (a generic parameter `it: I`) the argument's own type stands, unless the argument
        // takes it from where it goes (`x.into()`, `iter.collect()`, `Default::default()`).
        if arg.contains('(') {
            let ty = plain.get(i).and_then(|p| p.decl_ty.as_deref()).map(normalize_param_type_for_binding);
            let typed = ty.as_deref().is_some_and(is_writable_binding_type);
            let self_typed = ty.is_none() && !INFERRED_FROM_USE.iter().any(|w| contains_word(arg, w));
            let chained_temp = ty.as_deref().is_some_and(|t| t.starts_with('&')) && arg.matches('(').count() > 1;
            if !(typed || self_typed) || chained_temp || (self_kind.is_some() && contains_word(arg, "self")) {
                return None;
            }
        }
    }

    let prev_ch = masked[..call_start].trim_end().as_bytes().last().copied().unwrap_or(b'{');
    let next_ch = masked[call_end..].trim_start().as_bytes().first().copied().unwrap_or(b'}');
    let standalone = matches!(prev_ch, b'{' | b';') && matches!(next_ch, b';' | b'}');

    let inlined = format_inlined_call(recv_expr, self_kind, recv_ty, params, &raw_args, body_src, standalone, recv_in_match);
    Some((call_start, call_end, inlined))
}

/// Calls whose result type is chosen by where the value goes: an argument making one cannot be bound untyped.
const INFERRED_FROM_USE: &[&str] = &["into", "try_into", "collect", "parse", "default", "from", "try_from", "sum", "product", "unwrap_or_default", "new", "with_capacity"];

pub(crate) fn rewrite_self_to_this(body_src: &str, masked_body: &str) -> String {
    let mb = masked_body.as_bytes();
    let mut out = String::with_capacity(body_src.len() + 16);
    let mut i = 0usize;
    while i < mb.len() {
        if mb[i] == b's' && i + 4 <= mb.len() && &mb[i..i + 4] == b"self" {
            // `x.self` is no receiver; `0..self.n` (a range) is.
            let prev_ok = i == 0 || (!is_ident_byte(mb[i - 1]) && (mb[i - 1] != b'.' || (i >= 2 && mb[i - 2] == b'.')) && !(i >= 2 && &mb[i - 2..i] == b"::"));
            let next_ok = i + 4 == mb.len() || (!is_ident_byte(mb[i + 4]) && !(i + 6 <= mb.len() && &mb[i + 4..i + 6] == b"::"));
            if prev_ok && next_ok {
                out.push_str("this");
                i += 4;
                continue;
            }
        }
        let ch = body_src[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

pub(crate) fn rewrite_self_type_to_name(body_src: &str, masked_body: &str, type_name: &str) -> String {
    let mb = masked_body.as_bytes();
    let mut out = String::with_capacity(body_src.len() + 16);
    let mut i = 0usize;
    while i < mb.len() {
        if mb[i] == b'S' && i + 4 <= mb.len() && &mb[i..i + 4] == b"Self" {
            // `x.self` is no receiver; `0..self.n` (a range) is.
            let prev_ok = i == 0 || (!is_ident_byte(mb[i - 1]) && (mb[i - 1] != b'.' || (i >= 2 && mb[i - 2] == b'.')) && !(i >= 2 && &mb[i - 2..i] == b"::"));
            let next_ok = i + 4 == mb.len() || !is_ident_byte(mb[i + 4]);
            if prev_ok && next_ok {
                out.push_str(type_name);
                i += 4;
                continue;
            }
        }
        let ch = body_src[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

pub(crate) fn has_only_lifetime_generics(generics_src: &str) -> bool {
    let g = generics_src.trim();
    if g.is_empty() {
        return true;
    }
    let inner = g.strip_prefix('<').and_then(|s| s.strip_suffix('>')).unwrap_or(g);
    let parts: Vec<&str> = inner.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return true;
    }
    parts.iter().all(|p| p.starts_with('\''))
}

pub(crate) fn extract_type_generic_names(generics_src: &str) -> Vec<String> {
    let g = generics_src.trim();
    if g.is_empty() {
        return Vec::new();
    }
    let inner = g.strip_prefix('<').and_then(|s| s.strip_suffix('>')).unwrap_or(g);
    let parts: Vec<&str> = inner.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    let mut names = Vec::new();
    for p in parts {
        let trimmed = p.trim();
        if trimmed.starts_with('\'') {
            continue;
        }
        let raw_name = trimmed.split(':').next().unwrap_or(trimmed).trim();
        let name = raw_name.split('=').next().unwrap_or(raw_name).trim();
        if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            names.push(name.to_string());
        }
    }
    names
}

pub(crate) fn unwrap_tail_return(body_src: &str, masked_body: &str) -> Option<(String, String)> {
    let mb = masked_body.as_bytes();
    let mut depth = 0usize;
    let mut return_indices = Vec::new();
    let mut i = 0usize;
    while i < mb.len() {
        match mb[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b'r' if depth == 0 && i + 6 <= mb.len() && &mb[i..i + 6] == b"return" => {
                let prev_ok = i == 0 || (!is_ident_byte(mb[i - 1]) && mb[i - 1] != b'.');
                let next_ok = i + 6 == mb.len() || !is_ident_byte(mb[i + 6]);
                if prev_ok && next_ok {
                    return_indices.push(i);
                }
            }
            _ => {}
        }
        i += 1;
    }

    if return_indices.len() != 1 {
        return None;
    }
    let ret_byte = return_indices[0];

    let after_ret = &mb[ret_byte + 6..];
    let mut semi_pos = None;
    let mut sub_depth = 0usize;
    for (idx, &b) in after_ret.iter().enumerate() {
        match b {
            b'(' | b'[' | b'{' => sub_depth += 1,
            b')' | b']' | b'}' => sub_depth = sub_depth.saturating_sub(1),
            b';' if sub_depth == 0 => {
                semi_pos = Some(idx);
                break;
            }
            _ => {}
        }
    }

    if let Some(sp) = semi_pos {
        let rest = &after_ret[sp + 1..];
        if rest.iter().any(|b| !b.is_ascii_whitespace()) {
            return None;
        }
    }

    let before_body = &body_src[..ret_byte];
    let before_masked = &masked_body[..ret_byte];

    let expr_raw = if let Some(sp) = semi_pos { &body_src[ret_byte + 6..ret_byte + 6 + sp] } else { &body_src[ret_byte + 6..] };
    let expr_masked = if let Some(sp) = semi_pos { &masked_body[ret_byte + 6..ret_byte + 6 + sp] } else { &masked_body[ret_byte + 6..] };

    let trimmed_expr = expr_raw.trim();
    let trimmed_masked = expr_masked.trim();
    let (final_expr, final_masked) = if trimmed_expr.is_empty() { ("()", "()") } else { (trimmed_expr, trimmed_masked) };

    let new_body = format!("{before_body}{final_expr}");
    let new_masked = format!("{before_masked}{final_masked}");
    Some((new_body, new_masked))
}

pub(crate) fn find_enclosing_mod_range(masked: &str, byte_offset: usize) -> Option<(usize, usize)> {
    let mb = masked.as_bytes();
    let mut best: Option<(usize, usize)> = None;
    let mut p = 0;
    while p + 4 < mb.len() {
        if mb[p] == b'm' && &mb[p..p + 4] == b"mod " && (p == 0 || !is_ident_byte(mb[p - 1])) {
            let mut q = p + 4;
            while q < mb.len() && mb[q] != b'{' && mb[q] != b';' {
                q += 1;
            }
            if q < mb.len() && mb[q] == b'{' {
                if let Some(close) = skip_balanced(mb, q, b'{', b'}') {
                    if q < byte_offset && byte_offset < close && (best.is_none() || (close - q) < (best.unwrap().1 - best.unwrap().0)) {
                        best = Some((q, close));
                    }
                }
            }
        }
        p += 1;
    }
    best
}
