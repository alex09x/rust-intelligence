/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Parameter and receiver formatting helpers for `Inline into all callers`.

use super::action_generics_helpers::clean_param_ident;
use super::action_generics_sub::{split_top_level_colon, split_top_level_commas};
use super::action_parse::is_ident_byte;
use super::action_parse_decl::collapse_ws;

pub(crate) struct FnParamInfo {
    pub is_self: bool,
    pub self_kind: Option<SelfKind>,
    pub ident: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelfKind {
    Value,
    Ref,
    MutRef,
}

pub(crate) fn parse_fn_params(params_src: &str) -> Option<Vec<FnParamInfo>> {
    let mut out = Vec::new();
    for raw_p in split_top_level_commas(params_src) {
        let t = raw_p.trim();
        if t.is_empty() {
            continue;
        }
        if t == "self" || t == "mut self" {
            out.push(FnParamInfo { is_self: true, self_kind: Some(SelfKind::Value), ident: Some("this".to_string()) });
            continue;
        }
        if t == "&self" {
            out.push(FnParamInfo { is_self: true, self_kind: Some(SelfKind::Ref), ident: Some("this".to_string()) });
            continue;
        }
        if t == "&mut self" {
            out.push(FnParamInfo { is_self: true, self_kind: Some(SelfKind::MutRef), ident: Some("this".to_string()) });
            continue;
        }
        let (lhs, Some(rhs)) = split_top_level_colon(t) else { return None };
        let l_trim = lhs.trim();
        let (is_mut, raw_id) = if let Some(r) = l_trim.strip_prefix("mut ") { (true, r.trim()) } else { (false, l_trim) };
        let pid = clean_param_ident(raw_id)?;
        let pty = collapse_ws(rhs);
        if pty.is_empty() || pty.contains("impl ") || pty.contains('_') {
            return None;
        }
        let bind_lhs = if is_mut { format!("mut {pid}") } else { pid };
        out.push(FnParamInfo { is_self: false, self_kind: None, ident: Some(bind_lhs) });
    }
    Some(out)
}

pub(crate) fn rewrite_self_to_this(body_src: &str, masked_body: &str) -> String {
    let mb = masked_body.as_bytes();
    let mut out = String::with_capacity(body_src.len() + 16);
    let mut i = 0usize;
    while i < mb.len() {
        if mb[i] == b's' && i + 4 <= mb.len() && &mb[i..i + 4] == b"self" {
            let prev_ok = i == 0 || (!is_ident_byte(mb[i - 1]) && mb[i - 1] != b'.' && !(i >= 2 && &mb[i - 2..i] == b"::"));
            let next_ok = i + 4 == mb.len() || !is_ident_byte(mb[i + 4]);
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

pub(crate) fn find_receiver_start(_src: &str, masked: &str, dot_byte: usize) -> Option<usize> {
    let mb = masked.as_bytes();
    let mut p = dot_byte;
    while p > 0 && mb[p - 1].is_ascii_whitespace() {
        p -= 1;
    }
    if p == 0 {
        return None;
    }
    let last = mb[p - 1];
    if last == b')' {
        let open = skip_balanced_backwards(mb, p - 1, b')', b'(')?;
        let mut q = open;
        while q > 0 && mb[q - 1].is_ascii_whitespace() {
            q -= 1;
        }
        if q > 0 && (mb[q - 1].is_ascii_alphanumeric() || mb[q - 1] == b'_') {
            while q > 0 && (mb[q - 1].is_ascii_alphanumeric() || mb[q - 1] == b'_' || mb[q - 1] == b':') {
                q -= 1;
            }
        }
        return Some(q);
    }
    if last == b']' {
        let open = skip_balanced_backwards(mb, p - 1, b']', b'[')?;
        let mut q = open;
        while q > 0 && (mb[q - 1].is_ascii_alphanumeric() || mb[q - 1] == b'_' || mb[q - 1] == b'.') {
            q -= 1;
        }
        return Some(q);
    }
    if last == b'}' {
        return skip_balanced_backwards(mb, p - 1, b'}', b'{');
    }
    let mut q = p;
    while q > 0 {
        let b = mb[q - 1];
        if b.is_ascii_alphanumeric() || b == b'_' || b == b'.' {
            q -= 1;
        } else {
            break;
        }
    }
    if q < p {
        return Some(q);
    }
    None
}

fn skip_balanced_backwards(bytes: &[u8], close_idx: usize, close_ch: u8, open_ch: u8) -> Option<usize> {
    if bytes.get(close_idx)? != &close_ch {
        return None;
    }
    let mut depth = 0usize;
    let mut i = close_idx;
    loop {
        let b = bytes[i];
        if b == close_ch {
            depth += 1;
        } else if b == open_ch {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        if i == 0 {
            break;
        }
        i -= 1;
    }
    None
}

pub(crate) fn find_path_start(masked: &str, ref_byte: usize) -> usize {
    let before = &masked[..ref_byte];
    let trimmed = before.trim_end();
    if !trimmed.ends_with("::") {
        return ref_byte;
    }
    let mb = masked.as_bytes();
    let mut p = trimmed.len() - 2;
    loop {
        while p > 0 && mb[p - 1].is_ascii_whitespace() {
            p -= 1;
        }
        if p == 0 {
            return 0;
        }
        if mb[p - 1] == b'>' {
            if let Some(open) = skip_balanced_backwards(mb, p - 1, b'>', b'<') {
                p = open;
                while p > 0 && mb[p - 1].is_ascii_whitespace() {
                    p -= 1;
                }
                if p >= 2 && &mb[p - 2..p] == b"::" {
                    p -= 2;
                    while p > 0 && mb[p - 1].is_ascii_whitespace() {
                        p -= 1;
                    }
                }
            } else {
                break;
            }
        }
        let id_end = p;
        while p > 0 && (mb[p - 1].is_ascii_alphanumeric() || mb[p - 1] == b'_') {
            p -= 1;
        }
        if p == id_end {
            break;
        }
        let mut q = p;
        while q > 0 && mb[q - 1].is_ascii_whitespace() {
            q -= 1;
        }
        if q >= 2 && &mb[q - 2..q] == b"::" {
            p = q - 2;
        } else {
            break;
        }
    }
    p
}

pub(crate) fn format_inlined_call(recv_expr: Option<&str>, self_kind: Option<SelfKind>, params: &[FnParamInfo], args: &[String], body_src: &str, is_standalone: bool) -> String {
    let mut bindings = Vec::new();
    if let (Some(recv), Some(sk)) = (recv_expr, self_kind) {
        let trimmed_recv = recv.trim();
        let recv_binding = match sk {
            SelfKind::Value => format!("let this = {trimmed_recv};"),
            SelfKind::Ref => {
                if trimmed_recv == "self" {
                    "let this = &*self;".to_string()
                } else if trimmed_recv.starts_with('&') && !trimmed_recv.starts_with("&mut") {
                    format!("let this = {trimmed_recv};")
                } else {
                    format!("let this = &{trimmed_recv};")
                }
            }
            SelfKind::MutRef => {
                if trimmed_recv == "self" {
                    "let this = &mut *self;".to_string()
                } else if trimmed_recv.starts_with("&mut ") {
                    format!("let this = {trimmed_recv};")
                } else {
                    format!("let this = &mut {trimmed_recv};")
                }
            }
        };
        bindings.push(recv_binding);
    }

    let non_self_params: Vec<&FnParamInfo> = params.iter().filter(|p| !p.is_self).collect();
    for (p, arg) in non_self_params.iter().zip(args.iter()) {
        let arg_trimmed = arg.trim();
        if let Some(id) = &p.ident {
            bindings.push(format!("let {id} = {arg_trimmed};"));
        }
    }

    let stmts = if bindings.is_empty() { String::new() } else { format!("{} ", bindings.join(" ")) };

    let blk = if stmts.is_empty() { format!("{{ {body_src} }}") } else { format!("{{ {stmts}{body_src} }}") };

    if is_standalone {
        blk
    } else {
        format!("({blk})")
    }
}
