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
use super::action_inline_callers_helpers::{find_path_start, find_receiver_start, format_inlined_call, FnParamInfo, SelfKind};
use super::action_parse::is_ident_byte;
use super::action_parse_items::skip_balanced;

pub(crate) fn process_call_site(
    src: &str,
    masked: &str,
    ref_byte: usize,
    name: &str,
    self_kind: Option<SelfKind>,
    params: &[FnParamInfo],
    body_src: &str,
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
    if p >= src.len() || mb[p] != b'(' {
        return None;
    }
    let call_end = skip_balanced(mb, p, b'(', b')')?;
    let raw_args: Vec<String> = split_top_level_commas(&src[p + 1..call_end - 1]).into_iter().filter(|a| !a.trim().is_empty()).collect();

    let (call_start, recv_expr) = if self_kind.is_some() {
        let before_dot = src[..ref_byte].trim_end();
        if !before_dot.ends_with('.') || before_dot.ends_with("..") {
            return None;
        }
        let dot_byte = before_dot.len() - 1;
        let recv_start = find_receiver_start(src, masked, dot_byte)?;
        if recv_start > 0 && (mb[recv_start - 1] == b'.' || is_ident_byte(mb[recv_start - 1])) {
            return None;
        }
        let recv = src[recv_start..dot_byte].trim();
        if recv.is_empty() || recv.starts_with('.') || recv.ends_with('.') || recv.contains('(') || recv.contains('[') {
            return None;
        }
        (recv_start, Some(recv))
    } else {
        let path_start = find_path_start(masked, ref_byte);
        (path_start, None)
    };

    let non_self_count = params.iter().filter(|p| !p.is_self).count();
    if raw_args.len() != non_self_count || raw_args.iter().any(|a| a.contains('(')) {
        return None;
    }

    let prev_ch = masked[..call_start].trim_end().as_bytes().last().copied().unwrap_or(b'{');
    let next_ch = masked[call_end..].trim_start().as_bytes().first().copied().unwrap_or(b'}');
    let standalone = matches!(prev_ch, b'{' | b';') && matches!(next_ch, b';' | b'}');

    let inlined = format_inlined_call(recv_expr, self_kind, params, &raw_args, body_src, standalone);
    Some((call_start, call_end, inlined))
}

pub(crate) fn rewrite_self_to_this(body_src: &str, masked_body: &str) -> String {
    let mb = masked_body.as_bytes();
    let mut out = String::with_capacity(body_src.len() + 16);
    let mut i = 0usize;
    while i < mb.len() {
        if mb[i] == b's' && i + 4 <= mb.len() && &mb[i..i + 4] == b"self" {
            let prev_ok = i == 0 || (!is_ident_byte(mb[i - 1]) && mb[i - 1] != b'.' && !(i >= 2 && &mb[i - 2..i] == b"::"));
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
            let prev_ok = i == 0 || (!is_ident_byte(mb[i - 1]) && mb[i - 1] != b'.' && !(i >= 2 && &mb[i - 2..i] == b"::"));
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
