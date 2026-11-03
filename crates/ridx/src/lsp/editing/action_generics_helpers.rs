/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Receiver, lifetime-elision, and parameter-desugaring helpers for `action_generics`.

pub use super::action_generics_closure::{
    assemble_kept_generics_and_where, desugar_param_impl_trait, find_sibling_trait_defining_assoc, impl_body_defines_assoc_type, to_pascal_fn_alias, unique_ident, unique_lifetime,
};
use super::action_generics_sub::split_top_level_colon;
use super::action_parse::{is_ident_byte, mask_comments_and_strings};
use super::action_parse_decl::next_ident_token;
use super::action_parse_items::skip_balanced;
use ridx_format::{DefKind, WorkspaceIndex};
use std::path::Path;

#[derive(Debug, Clone)]
pub enum ReceiverLt {
    ByValue,
    UnlifetimedRef,
    ExplicitLt(String),
}

pub fn parse_receiver_param<F>(p: &str, sub_self: &F) -> Option<Option<(String, ReceiverLt)>>
where
    F: Fn(&str) -> Option<String>,
{
    let t = p.trim();
    if t == "self" || t == "mut self" {
        return Some(Some((sub_self("Self")?, ReceiverLt::ByValue)));
    }
    if let Some(rest) = t.strip_prefix('&') {
        let r = rest.trim_start();
        if r == "self" {
            return Some(Some((sub_self("&Self")?, ReceiverLt::UnlifetimedRef)));
        }
        if r == "mut self" {
            return Some(Some((sub_self("&mut Self")?, ReceiverLt::UnlifetimedRef)));
        }
        if r.starts_with('\'') {
            if let Some(sp) = r.find(char::is_whitespace) {
                let lt = &r[..sp];
                let after_lt = r[sp..].trim_start();
                if after_lt == "self" {
                    return Some(Some((sub_self(&format!("&{lt} Self"))?, ReceiverLt::ExplicitLt(lt.to_string()))));
                }
                if after_lt == "mut self" {
                    return Some(Some((sub_self(&format!("&{lt} mut Self"))?, ReceiverLt::ExplicitLt(lt.to_string()))));
                }
            }
        }
    }
    let (lhs, Some(rhs_ty)) = split_top_level_colon(t) else {
        return Some(None);
    };
    let lhs_clean = lhs.trim();
    if lhs_clean == "self" || lhs_clean == "mut self" {
        let rlt = if count_elided_lifetimes_outside_fn(rhs_ty) > 0 { ReceiverLt::UnlifetimedRef } else { ReceiverLt::ByValue };
        let subbed = sub_self(rhs_ty)?;
        return Some(Some((subbed, rlt)));
    }
    Some(None)
}

pub fn contains_inferred_underscore(s: &str) -> bool {
    let masked = mask_comments_and_strings(s);
    let b = masked.as_bytes();
    for i in 0..b.len() {
        if b[i] == b'_' {
            let prev_ok = i == 0 || (!is_ident_byte(b[i - 1]) && b[i - 1] != b'\'');
            let next_ok = i + 1 == b.len() || !is_ident_byte(b[i + 1]);
            if prev_ok && next_ok {
                return true;
            }
        }
    }
    false
}

pub fn count_elided_lifetimes_outside_fn(s: &str) -> usize {
    let masked = mask_comments_and_strings(s);
    let b = masked.as_bytes();
    let mut count = 0usize;
    let mut i = 0usize;
    while i < b.len() {
        if let Some(skip_to) = try_skip_fn_or_closure_trait(&masked, i) {
            i = skip_to;
            continue;
        }
        if i + 2 <= b.len() && &b[i..i + 2] == b"'_" && (i + 2 == b.len() || !is_ident_byte(b[i + 2])) {
            count += 1;
            i += 2;
        } else if b[i] == b'&' {
            let mut k = i + 1;
            while k < b.len() && b[k].is_ascii_whitespace() {
                k += 1;
            }
            if b.get(k).copied() != Some(b'\'') {
                count += 1;
            }
            i += 1;
        } else {
            i += 1;
        }
    }
    count
}

pub fn bind_first_ref_lifetime(param_str: &str, lt: &str) -> String {
    let b = param_str.as_bytes();
    for i in 0..b.len() {
        if b[i] == b'&' {
            let mut k = i + 1;
            while k < b.len() && b[k].is_ascii_whitespace() {
                k += 1;
            }
            if b.get(k).copied() != Some(b'\'') {
                return format!("{}&{lt} {}", &param_str[..i], &param_str[k..]);
            }
        }
    }
    param_str.to_string()
}

pub fn bind_elided_lifetimes_outside_fn(s: &str, lt: &str) -> String {
    let masked = mask_comments_and_strings(s);
    let b = masked.as_bytes();
    let mut out = String::with_capacity(s.len() + 16);
    let mut i = 0usize;
    while i < b.len() {
        if let Some(skip_to) = try_skip_fn_or_closure_trait(&masked, i) {
            out.push_str(&s[i..skip_to]);
            i = skip_to;
            continue;
        }
        if i + 2 <= b.len() && &b[i..i + 2] == b"'_" && (i + 2 == b.len() || !is_ident_byte(b[i + 2])) {
            out.push_str(lt);
            i += 2;
        } else if b[i] == b'&' {
            let mut k = i + 1;
            while k < b.len() && b[k].is_ascii_whitespace() {
                k += 1;
            }
            if b.get(k).copied() == Some(b'\'') {
                out.push('&');
                i += 1;
            } else {
                out.push('&');
                out.push_str(lt);
                out.push(' ');
                i = k;
            }
        } else {
            let ch = s[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

fn try_skip_fn_or_closure_trait(masked: &str, i: usize) -> Option<usize> {
    let b = masked.as_bytes();
    if i > 0 && is_ident_byte(b[i - 1]) {
        return None;
    }
    let (tok, end_tok) = next_ident_token(masked, i)?;
    if !matches!(tok, "fn" | "Fn" | "FnMut" | "FnOnce") || i + tok.len() != end_tok {
        return None;
    }
    let mut k = end_tok;
    while k < b.len() && b[k].is_ascii_whitespace() {
        k += 1;
    }
    let paren_end = skip_balanced(b, k, b'(', b')')?;
    let mut m = paren_end;
    while m < b.len() && b[m].is_ascii_whitespace() {
        m += 1;
    }
    if m + 2 <= b.len() && &b[m..m + 2] == b"->" {
        let mut p = m + 2;
        let (mut dp, mut db, mut da) = (0i32, 0i32, 0i32);
        while p < b.len() {
            let prev = if p > 0 { b[p - 1] } else { 0 };
            match b[p] {
                b'(' => dp += 1,
                b')' if dp == 0 => break,
                b')' => dp -= 1,
                b'[' => db += 1,
                b']' if db == 0 => break,
                b']' => db -= 1,
                b'<' if dp == 0 && db == 0 => da += 1,
                b'>' if dp == 0 && db == 0 && prev != b'-' && prev != b'=' => {
                    if da == 0 {
                        break;
                    }
                    da -= 1;
                }
                b',' | b'+' if dp == 0 && db == 0 && da == 0 => break,
                _ => {}
            }
            p += 1;
        }
        return Some(p);
    }
    Some(paren_end)
}

pub fn clean_param_ident(pat_raw: &str) -> Option<String> {
    let mut s = pat_raw.trim();
    while let Some(r) = s.strip_prefix("mut ").or_else(|| s.strip_prefix("ref ")) {
        s = r.trim_start();
    }
    if s.is_empty() || s == "_" {
        return None;
    }
    let bare = s.strip_prefix("r#").unwrap_or(s);
    let bytes = bare.as_bytes();
    if !(bytes[0].is_ascii_alphabetic() || bytes[0] == b'_') || !bytes.iter().all(|&b| is_ident_byte(b)) {
        return None;
    }
    const STRICT_KWS: &[&str] = &[
        "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod",
        "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while",
    ];
    if s == bare && STRICT_KWS.contains(&s) {
        return None;
    }
    Some(s.to_string())
}

pub fn count_explicit_lifetimes_outside_fn(s: &str) -> usize {
    let masked = mask_comments_and_strings(s);
    let b = masked.as_bytes();
    let mut count = 0usize;
    let mut i = 0usize;
    while i < b.len() {
        if let Some(skip_to) = try_skip_fn_or_closure_trait(&masked, i) {
            i = skip_to;
            continue;
        }
        if b[i] == b'\'' && i + 1 < b.len() && (b[i + 1].is_ascii_alphabetic() || b[i + 1] == b'_') {
            let mut end = i + 2;
            while end < b.len() && is_ident_byte(b[end]) {
                end += 1;
            }
            if &b[i..end] != b"'_" {
                count += 1;
            }
            i = end;
        } else {
            i += 1;
        }
    }
    count
}

pub fn has_unbracketed_lifetime_nominal(s: &str, masked_doc: &str, current_file: &str, type_params: &std::collections::HashSet<String>, ws: &WorkspaceIndex) -> bool {
    let masked = mask_comments_and_strings(s);
    let b = masked.as_bytes();
    let cur_path = Path::new(current_file);
    let mut prev_tok = "";
    let mut i = 0usize;
    while i < b.len() {
        if let Some(skip_to) = try_skip_fn_or_closure_trait(&masked, i) {
            prev_tok = "";
            i = skip_to;
            continue;
        }
        if (i == 0 || (!is_ident_byte(b[i - 1]) && b[i - 1] != b'\'')) && (b[i].is_ascii_alphabetic() || b[i] == b'_') {
            let Some((raw_tok, end_tok)) = next_ident_token(&masked, i) else {
                i += 1;
                continue;
            };
            let tok = raw_tok.strip_prefix("r#").unwrap_or(raw_tok);
            let after = masked[end_tok..].trim_start();
            if tok != "Self"
                && prev_tok != "dyn"
                && !type_params.contains(tok)
                && tok.as_bytes().first().is_some_and(u8::is_ascii_uppercase)
                && !after.starts_with('<')
                && !after.starts_with("::")
                && nominal_declares_lifetime(tok, masked_doc, cur_path, ws)
            {
                return true;
            }
            prev_tok = tok;
            i = end_tok;
            continue;
        }
        i += 1;
    }
    false
}

fn nominal_declares_lifetime(ident: &str, masked_doc: &str, cur_path: &Path, ws: &WorkspaceIndex) -> bool {
    for kw in ["struct", "enum", "union", "type"] {
        let pat = format!("{kw} {ident}");
        for (idx, _) in masked_doc.match_indices(&pat) {
            let prev_ok = idx == 0 || !is_ident_byte(masked_doc.as_bytes()[idx - 1]);
            let after_idx = idx + pat.len();
            let next_ok = after_idx == masked_doc.len() || !is_ident_byte(masked_doc.as_bytes()[after_idx]);
            if prev_ok && next_ok {
                let rest = masked_doc[after_idx..].trim_start();
                return rest.strip_prefix('<').is_some_and(|g| g.trim_start().starts_with('\''));
            }
        }
    }
    let cur_manifest = cargo_manifest_dir(cur_path);
    let mut all_ws = Vec::new();
    let mut same_crate = Vec::new();
    for sym in ws.exact_local_symbols_by_name(ident) {
        let rp = sym.resolved_path();
        if !matches!(sym.kind, DefKind::Struct | DefKind::Enum | DefKind::Union | DefKind::TyAlias)
            || sym.is_macro_expansion
            || sym.name.strip_prefix("r#").unwrap_or(sym.name) != ident
        {
            continue;
        }
        let Some(sig) = sym.signature else { continue };
        let has_lt = sig_declares_lifetime_param(sig, ident);
        if cur_manifest.is_some() && cargo_manifest_dir(&rp) == cur_manifest {
            same_crate.push(has_lt);
        }
        all_ws.push(has_lt);
    }
    let pool = if !same_crate.is_empty() { &same_crate } else { &all_ws };
    !pool.is_empty() && pool.iter().all(|&b| b)
}

fn cargo_manifest_dir(p: &Path) -> Option<&Path> {
    let mut cur = p.parent();
    while let Some(dir) = cur {
        if dir.join("Cargo.toml").is_file() {
            return Some(dir);
        }
        cur = dir.parent();
    }
    None
}

fn sig_declares_lifetime_param(sig: &str, ident: &str) -> bool {
    let Some(pos) = sig.find(ident) else { return false };
    let rest = sig[pos + ident.len()..].trim_start();
    rest.strip_prefix('<').is_some_and(|g| g.trim_start().starts_with('\''))
}
