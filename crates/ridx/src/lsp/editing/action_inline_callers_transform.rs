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
use super::action_inline_callers::line_col_to_byte;
use super::action_inline_callers_helpers::{find_path_start, find_receiver_start, format_inlined_call, is_caller_return_compatible, FnParamInfo, SelfKind};
use super::action_parse::{is_ident_byte, line_starts_of, mask_comments_and_strings};
use super::action_parse_items::skip_balanced;
use super::action_struct_conv_helpers::to_lsp_edits;
use ridx_format::WorkspaceIndex;
use serde_json::Value;
use std::path::Path;

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
    let mut has_turbofish = false;
    if p + 2 < src.len() && &src[p..p + 2] == "::" {
        let mut q = p + 2;
        while q < src.len() && mb[q].is_ascii_whitespace() {
            q += 1;
        }
        if q < src.len() && mb[q] == b'<' {
            if let Some(close) = skip_balanced(mb, q, b'<', b'>') {
                has_turbofish = true;
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
    if has_turbofish && !body_src.trim().is_empty() {
        return None;
    }
    let call_end = skip_balanced(mb, p, b'(', b')')?;
    let raw_args: Vec<String> = split_top_level_commas(&src[p + 1..call_end - 1]).into_iter().filter(|a| !a.trim().is_empty()).collect();

    let (call_start, recv_expr, recv_ty) = if self_kind.is_some() {
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

    if is_fully_pub {
        if self_kind == Some(SelfKind::MutRef) {
            let recv_str = recv_expr.unwrap_or("");
            let is_mut_ref = recv_ty.is_some_and(|t| t.starts_with("&mut "));
            let is_explicit_mut = recv_str.starts_with("&mut ") || recv_str == "self" || recv_str == "this";
            if !is_mut_ref && !is_explicit_mut {
                return None;
            }
        } else if self_kind == Some(SelfKind::Value) && recv_ty.is_none() {
            return None;
        }
    }

    let non_self_count = params.iter().filter(|p| !p.is_self).count();
    if raw_args.len() != non_self_count || raw_args.iter().any(|a| a.contains('(')) {
        return None;
    }

    let prev_ch = masked[..call_start].trim_end().as_bytes().last().copied().unwrap_or(b'{');
    let next_ch = masked[call_end..].trim_start().as_bytes().first().copied().unwrap_or(b'}');
    let standalone = matches!(prev_ch, b'{' | b';') && matches!(next_ch, b';' | b'}');

    let inlined = format_inlined_call(recv_expr, self_kind, recv_ty, params, &raw_args, body_src, standalone);
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn process_call_sites_for_document(
    doc: &str,
    masked: &str,
    starts: &[usize],
    spans: &[(u32, u32)],
    fn_name: &str,
    self_kind: Option<SelfKind>,
    params: &[FnParamInfo],
    body_src: &str,
    body_has_question_mark: bool,
    effective_ret_ty: Option<&str>,
    def_range: Option<(usize, usize)>,
    file_opt: Option<&str>,
    ws_opt: Option<&WorkspaceIndex>,
    is_fully_pub: bool,
) -> (Vec<(usize, usize, String)>, bool) {
    let mut edits = Vec::new();
    let mut any_skipped = false;
    for &(span_line, span_col) in spans {
        let Some(ref_byte) = line_col_to_byte(doc, starts, span_line, span_col) else {
            any_skipped = true;
            continue;
        };
        if let Some((def_lo, def_hi)) = def_range {
            if ref_byte >= def_lo && ref_byte < def_hi {
                continue;
            }
        }
        if body_has_question_mark {
            let ret = effective_ret_ty.unwrap_or("");
            if !is_caller_return_compatible(doc, masked, starts, ref_byte, ret) {
                any_skipped = true;
                continue;
            }
        }
        if let Some((site_start, site_end, inlined_text)) = process_call_site(doc, masked, starts, ref_byte, fn_name, self_kind, params, body_src, file_opt, ws_opt, is_fully_pub) {
            edits.push((site_start, site_end, inlined_text));
        } else {
            any_skipped = true;
        }
    }
    (edits, any_skipped)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn process_other_document_call_sites(
    abs_path: &Path,
    spans: &[(u32, u32)],
    fn_name: &str,
    self_kind: Option<SelfKind>,
    params: &[FnParamInfo],
    body_src: &str,
    body_has_question_mark: bool,
    effective_ret_ty: Option<&str>,
    ws_opt: Option<&WorkspaceIndex>,
    is_fully_pub: bool,
) -> Option<Vec<Value>> {
    let other_doc = std::fs::read_to_string(abs_path).ok()?;
    let other_masked = mask_comments_and_strings(&other_doc);
    let other_starts = line_starts_of(&other_doc);
    let (edits, skipped) = process_call_sites_for_document(
        &other_doc,
        &other_masked,
        &other_starts,
        spans,
        fn_name,
        self_kind,
        params,
        body_src,
        body_has_question_mark,
        effective_ret_ty,
        None,
        abs_path.to_str(),
        ws_opt,
        is_fully_pub,
    );
    if skipped || edits.is_empty() {
        return None;
    }
    to_lsp_edits(&other_doc, &other_starts, edits)
}
