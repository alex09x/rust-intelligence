/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Generic alpha-renaming (`E0403`) and UFCS self-call qualification (`E0034`) for default trait members.

use super::action_generics_sub::{split_top_level_colon, split_top_level_commas, strip_leading_attrs_keep_cfg, GenKind, GenericDecl};
use super::action_parse::{is_ident_byte, mask_comments_and_strings};
use super::action_parse_decl::{extract_angle_generics, find_header_terminator, next_ident_token};
use super::action_parse_items::{contains_word, skip_balanced};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraitFnRecv {
    AssocFn,
    Ref,
    RefMut,
    Owned { is_mut: bool },
    Custom,
}

pub fn parse_fn_receiver(params_masked: &str) -> TraitFnRecv {
    let Some(first) = split_top_level_commas(params_masked).into_iter().next() else {
        return TraitFnRecv::AssocFn;
    };
    let (_, clean) = strip_leading_attrs_keep_cfg(&first);
    let t = clean.trim();
    if t.is_empty() {
        return TraitFnRecv::AssocFn;
    }
    if t == "self" {
        return TraitFnRecv::Owned { is_mut: false };
    }
    if t == "mut self" {
        return TraitFnRecv::Owned { is_mut: true };
    }
    if let Some(rest) = t.strip_prefix('&') {
        let mut r = rest.trim_start();
        if r.starts_with('\'') {
            if let Some(sp) = r.find(char::is_whitespace) {
                r = r[sp..].trim_start();
            }
        }
        if r == "self" {
            return TraitFnRecv::Ref;
        }
        if r == "mut self" {
            return TraitFnRecv::RefMut;
        }
    }
    let (lhs, rhs) = split_top_level_colon(t);
    if rhs.is_some() && matches!(lhs.trim(), "self" | "mut self") {
        return TraitFnRecv::Custom;
    }
    TraitFnRecv::AssocFn
}

pub fn collect_trait_fn_receivers(body_src: &str, body_masked: &str) -> HashMap<String, TraitFnRecv> {
    let mb = body_masked.as_bytes();
    let mut out = HashMap::new();
    let mut i = 0usize;
    while i < mb.len() {
        while i < mb.len() && mb[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= mb.len() {
            break;
        }
        while mb.get(i).copied() == Some(b'#') {
            let mut k = i + 1;
            if mb.get(k).copied() == Some(b'!') {
                k += 1;
            }
            while k < mb.len() && mb[k].is_ascii_whitespace() {
                k += 1;
            }
            if mb.get(k).copied() == Some(b'[') {
                let Some(after_a) = skip_balanced(mb, k, b'[', b']') else {
                    return out;
                };
                i = after_a;
                while i < mb.len() && mb[i].is_ascii_whitespace() {
                    i += 1;
                }
            } else {
                break;
            }
        }
        let mut pos = i;
        while let Some((tok, next_p)) = next_ident_token(body_masked, pos) {
            if matches!(tok, "async" | "const" | "unsafe" | "extern") {
                pos = next_p;
                let trimmed = body_masked[pos..].trim_start();
                if let Some(after_open_q) = trimmed.strip_prefix('"') {
                    let ws_len = body_masked[pos..].len() - trimmed.len();
                    if let Some(end_q) = after_open_q.find('"') {
                        pos += ws_len + end_q + 2;
                    }
                }
            } else {
                break;
            }
        }
        let Some((kw, kw_end)) = next_ident_token(body_masked, pos) else {
            i += 1;
            continue;
        };
        if kw == "fn" {
            let Some((raw_n, name_end)) = next_ident_token(body_src, kw_end) else {
                break;
            };
            let (_, after_g) = extract_angle_generics(body_src, body_masked, name_end);
            let mut p = after_g;
            while p < mb.len() && mb[p].is_ascii_whitespace() {
                p += 1;
            }
            let Some(paren_end) = skip_balanced(mb, p, b'(', b')') else {
                break;
            };
            let params_masked = &body_masked[p + 1..paren_end - 1];
            out.insert(raw_n.strip_prefix("r#").unwrap_or(raw_n).to_string(), parse_fn_receiver(params_masked));
            let Some(term) = find_header_terminator(mb, paren_end) else {
                break;
            };
            i = if mb[term] == b'{' {
                let Some(after_b) = skip_balanced(mb, term, b'{', b'}') else {
                    break;
                };
                after_b
            } else {
                term + 1
            };
            continue;
        }
        let Some(term) = find_header_terminator(mb, kw_end) else {
            break;
        };
        i = if mb[term] == b'{' {
            let Some(after_b) = skip_balanced(mb, term, b'{', b'}') else {
                break;
            };
            after_b
        } else {
            term + 1
        };
    }
    out
}

pub fn alpha_rename_colliding_fn_generics<F>(
    cur_src: &str,
    fn_gens: &[GenericDecl],
    fn_gen_names: &HashSet<String>,
    impl_non_lt_gens: &HashSet<String>,
    doc_text: &str,
    replace_ident: F,
) -> Option<(String, HashSet<String>)>
where
    F: Fn(&str, &str, &str) -> String,
{
    let mut out_src = cur_src.to_string();
    let mut eff_names = fn_gen_names.clone();
    for g in fn_gens {
        if g.kind == GenKind::Lifetime || !impl_non_lt_gens.contains(&g.name) {
            continue;
        }
        let masked_cur = mask_comments_and_strings(&out_src);
        if masked_cur.contains(&format!("Self::{}", g.name)) {
            return None;
        }
        let mut idx = 0usize;
        let fresh = loop {
            let cand = format!("{}{idx}", g.name);
            if !impl_non_lt_gens.contains(&cand) && !eff_names.contains(&cand) && !contains_word(&masked_cur, &cand) && !contains_word(doc_text, &cand) {
                break cand;
            }
            idx += 1;
        };
        out_src = replace_ident(&out_src, &g.name, &fresh);
        eff_names.remove(&g.name);
        eff_names.insert(fresh);
    }
    Some((out_src, eff_names))
}

pub fn qualify_trait_self_calls(
    fn_src: &str,
    trait_ty: &str,
    caller_recv: TraitFnRecv,
    trait_receivers: &HashMap<String, TraitFnRecv>,
    colliding_methods: &HashSet<String>,
) -> Option<String> {
    if colliding_methods.is_empty() {
        return Some(fn_src.to_string());
    }
    let masked = mask_comments_and_strings(fn_src);
    let mb = masked.as_bytes();
    // An associated const has no body calling methods: nothing to qualify.
    let Some(kw_pos) = find_fn_kw(&masked) else { return Some(fn_src.to_string()) };
    let (_, name_end) = next_ident_token(fn_src, kw_pos + 2)?;
    let (_, after_g) = extract_angle_generics(fn_src, &masked, name_end);
    let mut p = after_g;
    while p < mb.len() && mb[p].is_ascii_whitespace() {
        p += 1;
    }
    let paren_end = skip_balanced(mb, p, b'(', b')')?;
    let body_open = find_header_terminator(mb, paren_end)?;
    if mb.get(body_open).copied() != Some(b'{') {
        return Some(fn_src.to_string());
    }
    let after_body = skip_balanced(mb, body_open, b'{', b'}')?;
    let body_close = after_body - 1;
    let body_masked_init = &masked[body_open + 1..body_close];
    if !colliding_methods.iter().any(|m| contains_word(body_masked_init, m)) {
        return Some(fn_src.to_string());
    }
    if contains_word(body_masked_init, "impl") {
        return None;
    }

    let clean_trait = trait_ty.trim();
    let mut cur_body = fn_src[body_open + 1..body_close].to_string();
    for _ in 0..32 {
        let bm = mask_comments_and_strings(&cur_body);
        let Some((rep_lo, rep_hi, replacement)) = find_rightmost_self_call(&cur_body, &bm, clean_trait, caller_recv, trait_receivers, colliding_methods)? else {
            return Some(format!("{}{}{}", &fn_src[..body_open + 1], cur_body, &fn_src[body_close..]));
        };
        cur_body.replace_range(rep_lo..rep_hi, &replacement);
    }
    Some(format!("{}{}{}", &fn_src[..body_open + 1], cur_body, &fn_src[body_close..]))
}

fn find_fn_kw(masked: &str) -> Option<usize> {
    let b = masked.as_bytes();
    let mut i = 0usize;
    while i + 2 <= b.len() {
        if &b[i..i + 2] == b"fn" {
            let prev_ok = i == 0 || !is_ident_byte(b[i - 1]);
            let next_ok = i + 2 == b.len() || !is_ident_byte(b[i + 2]);
            if prev_ok && next_ok {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

type SelfCallRewrite = (usize, usize, String);

fn find_rightmost_self_call(
    body_src: &str,
    body_masked: &str,
    clean_trait: &str,
    caller_recv: TraitFnRecv,
    trait_receivers: &HashMap<String, TraitFnRecv>,
    colliding_methods: &HashSet<String>,
) -> Option<Option<SelfCallRewrite>> {
    let b = body_masked.as_bytes();
    let mut last_match: Option<SelfCallRewrite> = None;
    let mut i = 0usize;
    while i + 4 <= b.len() {
        let prev_ok = i == 0 || (!is_ident_byte(b[i - 1]) && b[i - 1] != b'.' && !(i >= 2 && &b[i - 2..i] == b"::"));
        let next_ok = i + 4 == b.len() || !is_ident_byte(b[i + 4]);
        if prev_ok && next_ok && &b[i..i + 4] == b"Self" {
            let mut k = i + 4;
            while k < b.len() && b[k].is_ascii_whitespace() {
                k += 1;
            }
            if k + 2 <= b.len() && &b[k..k + 2] == b"::" {
                if let Some((raw_m, m_end)) = next_ident_token(body_masked, k + 2) {
                    let m = raw_m.strip_prefix("r#").unwrap_or(raw_m);
                    if colliding_methods.contains(m) && trait_receivers.contains_key(m) {
                        let tail = body_masked[m_end..].trim_start();
                        if tail.starts_with('(') || tail.starts_with("::<") {
                            last_match = Some((i, m_end, format!("<Self as {clean_trait}>::{raw_m}")));
                        }
                    }
                }
            }
        } else if prev_ok && next_ok && &b[i..i + 4] == b"self" {
            let mut k = i + 4;
            while k < b.len() && b[k].is_ascii_whitespace() {
                k += 1;
            }
            if b.get(k).copied() == Some(b'.') && b.get(k + 1).copied() != Some(b'.') {
                if let Some((raw_m, m_end)) = next_ident_token(body_masked, k + 1) {
                    let m = raw_m.strip_prefix("r#").unwrap_or(raw_m);
                    if let Some(&callee_recv) = colliding_methods.contains(m).then(|| trait_receivers.get(m)).flatten() {
                        let mut p = m_end;
                        while p < b.len() && b[p].is_ascii_whitespace() {
                            p += 1;
                        }
                        let mut tf = "";
                        if p + 2 <= b.len() && &b[p..p + 2] == b"::" {
                            let mut q = p + 2;
                            while q < b.len() && b[q].is_ascii_whitespace() {
                                q += 1;
                            }
                            if b.get(q).copied() == Some(b'<') {
                                let after_tf = skip_balanced(b, q, b'<', b'>')?;
                                tf = &body_src[p..after_tf];
                                p = after_tf;
                                while p < b.len() && b[p].is_ascii_whitespace() {
                                    p += 1;
                                }
                            }
                        }
                        if b.get(p).copied() == Some(b'(') {
                            let close_after = skip_balanced(b, p, b'(', b')')?;
                            let recv_expr = match (caller_recv, callee_recv) {
                                (TraitFnRecv::Ref, TraitFnRecv::Ref) => "self",
                                (TraitFnRecv::RefMut, TraitFnRecv::Ref) => "&*self",
                                (TraitFnRecv::RefMut, TraitFnRecv::RefMut) => "&mut *self",
                                (TraitFnRecv::Owned { .. }, TraitFnRecv::Ref) => "&self",
                                (TraitFnRecv::Owned { is_mut: true }, TraitFnRecv::RefMut) => "&mut self",
                                (TraitFnRecv::Owned { .. }, TraitFnRecv::Owned { .. }) => "self",
                                (TraitFnRecv::Custom, TraitFnRecv::Custom) => {
                                    i += 4;
                                    continue;
                                }
                                _ => return None,
                            };
                            let args_src = &body_src[p + 1..close_after - 1];
                            let args_masked = &body_masked[p + 1..close_after - 1];
                            let rep = if args_masked.trim().is_empty() {
                                format!("<Self as {clean_trait}>::{raw_m}{tf}({recv_expr})")
                            } else {
                                format!("<Self as {clean_trait}>::{raw_m}{tf}({recv_expr}, {args_src})")
                            };
                            last_match = Some((i, close_after, rep));
                        }
                    }
                }
            }
        }
        i += 1;
    }
    Some(last_match)
}
