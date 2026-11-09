/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Bounded matcher and transcriber for [`super::mbe_parse::MacroRulesDef`].

use super::mbe_parse::{matching_close, take_delimited, FragKind, MacroRulesDef, PatElem, RepOp, TplElem};
use crate::lexer::lex;
use crate::token::{Token, TokenKind as T};

pub const MAX_MBE_DEPTH: u32 = 12;
const MAX_EXPANDED_BYTES: usize = 131_072;

/// Rigid byte-offset mapping from an expanded `$var` span `[exp_start..exp_end)` back to `[orig_start..)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExpandedSpanMap {
    pub exp_start: u32,
    pub exp_end: u32,
    pub orig_start: u32,
}

#[inline]
pub fn map_span_range(spans: &[ExpandedSpanMap], exp_s: u32, exp_e: u32) -> Option<(u32, u32)> {
    let m = (exp_e >= exp_s).then(|| spans.iter().find(|m| exp_s >= m.exp_start && exp_e <= m.exp_end))??;
    let orig_s = m.orig_start + (exp_s - m.exp_start);
    Some((orig_s, orig_s + (exp_e - exp_s)))
}

#[derive(Clone, Debug)]
enum BoundVal<'s> {
    Single(&'s str, u32),
    Seq(Vec<BoundVal<'s>>),
}

type Bindings<'s> = Vec<(String, BoundVal<'s>)>;

impl MacroRulesDef {
    /// Attempts to expand `call_inner_src` (the inner text of `foo! { ... }` / `foo!(...)`).
    pub fn expand(&self, call_inner_src: &str, crate_prefix: &str) -> Option<String> {
        self.expand_with_spans(call_inner_src, 0, crate_prefix, &mut Vec::new())
    }

    /// Expands `call_inner_src` (starting at byte `call_base_off` in the enclosing file) and records
    /// rigid byte-offset mappings for every transcribed metavariable fragment in `spans_out`.
    pub fn expand_with_spans(&self, call_inner_src: &str, call_base_off: u32, crate_prefix: &str, spans_out: &mut Vec<ExpandedSpanMap>) -> Option<String> {
        let toks = lex(call_inner_src);
        let sig: Vec<Token> = toks.into_iter().filter(|t| !t.is_trivia()).collect();
        for (pat, tpl) in &self.arms {
            let mut binds = Bindings::new();
            if match_seq(call_inner_src, call_base_off, &sig, 0, pat, None, None, &mut binds) == Some(sig.len()) {
                let mut out = String::new();
                spans_out.clear();
                if transcribe_seq(tpl, &binds, crate_prefix, &mut out, spans_out) && out.len() <= MAX_EXPANDED_BYTES {
                    return Some(out);
                }
            }
        }
        spans_out.clear();
        None
    }
}

fn collect_pat_vars(pats: &[PatElem], out: &mut Vec<String>) {
    for p in pats {
        match p {
            PatElem::Var { name, .. } => out.push(name.clone()),
            PatElem::Group { inner, .. } | PatElem::Rep { inner, .. } => collect_pat_vars(inner, out),
            PatElem::Lit(..) => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn match_seq<'s>(
    src: &'s str,
    base_off: u32,
    sig: &[Token],
    mut pos: usize,
    pats: &[PatElem],
    rep_sep: Option<(&str, T)>,
    after_rep: Option<&PatElem>,
    binds: &mut Bindings<'s>,
) -> Option<usize> {
    for (pi, pat) in pats.iter().enumerate() {
        let next_p = pats.get(pi + 1);
        match pat {
            PatElem::Lit(txt, kind) => {
                let tok = sig.get(pos)?;
                if tok.kind != *kind || tok.text(src) != txt {
                    return None;
                }
                pos += 1;
            }
            PatElem::Group { open, inner } => {
                if sig.get(pos)?.kind != *open {
                    return None;
                }
                let (grp_inner, after) = take_delimited(sig, pos)?;
                if match_seq(src, base_off, grp_inner, 0, inner, None, None, binds) != Some(grp_inner.len()) {
                    return None;
                }
                pos = after;
            }
            PatElem::Var { name, frag } => {
                let tail_sep = if next_p.is_none() { rep_sep } else { None };
                let tail_after = if next_p.is_none() { after_rep } else { None };
                let end = match_frag(src, sig, pos, *frag, next_p, tail_sep, tail_after)?;
                let (slice_str, orig_s) =
                    if pos == end { ("", base_off) } else { (&src[sig[pos].start as usize..sig[end - 1].end as usize], base_off.wrapping_add(sig[pos].start)) };
                binds.push((name.clone(), BoundVal::Single(slice_str, orig_s)));
                pos = end;
            }
            PatElem::Rep { inner, sep, op } => {
                let mut vars = Vec::new();
                collect_pat_vars(inner, &mut vars);
                let mut per_var: Vec<Vec<BoundVal<'s>>> = vec![Vec::new(); vars.len()];
                let mut count = 0usize;
                let sep_ref = sep.as_ref().map(|(s, k)| (s.as_str(), *k));
                let eff_after = next_p.or(after_rep);
                loop {
                    if *op == RepOp::ZeroOrOne && count == 1 {
                        break;
                    }
                    let mut iter_pos = pos;
                    if count > 0 {
                        if let Some((stxt, skind)) = sep_ref {
                            let Some(t) = sig.get(iter_pos) else { break };
                            if t.kind != skind || t.text(src) != stxt {
                                break;
                            }
                            iter_pos += 1;
                        }
                    }
                    let mut iter_binds = Bindings::new();
                    let Some(next_pos) = match_seq(src, base_off, sig, iter_pos, inner, sep_ref, eff_after, &mut iter_binds) else {
                        break;
                    };
                    if next_pos == pos {
                        break;
                    }
                    for (vi, vname) in vars.iter().enumerate() {
                        if let Some((_, val)) = iter_binds.iter().find(|(n, _)| n == vname) {
                            per_var[vi].push(val.clone());
                        }
                    }
                    pos = next_pos;
                    count += 1;
                }
                if *op == RepOp::OneOrMore && count == 0 {
                    return None;
                }
                for (vname, seq) in vars.into_iter().zip(per_var) {
                    binds.push((vname, BoundVal::Seq(seq)));
                }
            }
        }
    }
    Some(pos)
}

fn match_frag(src: &str, sig: &[Token], pos: usize, frag: FragKind, next_pat: Option<&PatElem>, rep_sep: Option<(&str, T)>, after_rep: Option<&PatElem>) -> Option<usize> {
    match frag {
        FragKind::Vis => {
            if sig.get(pos).map(|t| t.kind) != Some(T::Pub) {
                return Some(pos);
            }
            if sig.get(pos + 1).map(|t| t.kind) == Some(T::LParen) {
                Some(take_delimited(sig, pos + 1)?.1)
            } else {
                Some(pos + 1)
            }
        }
        FragKind::Tt => {
            let tok = sig.get(pos)?;
            if matching_close(tok.kind).is_some() {
                Some(take_delimited(sig, pos)?.1)
            } else {
                (!matches!(tok.kind, T::RParen | T::RBracket | T::RBrace)).then_some(pos + 1)
            }
        }
        FragKind::Ident => {
            let tok = sig.get(pos)?;
            (matches!(tok.kind, T::Ident | T::RawIdent) || (tok.kind.is_keyword() && tok.kind != T::Crate)).then_some(pos + 1)
        }
        FragKind::Block => (sig.get(pos)?.kind == T::LBrace).then(|| take_delimited(sig, pos)).flatten().map(|(_, a)| a),
        FragKind::Literal => {
            let i = if sig.get(pos).map(|t| t.kind) == Some(T::Minus) { pos + 1 } else { pos };
            let t = sig.get(i)?;
            (t.kind.is_literal() || matches!(t.kind, T::True | T::False)).then_some(i + 1)
        }
        FragKind::Meta => match_meta(sig, pos),
        FragKind::Path => match_path(sig, pos),
        FragKind::Item => match_item(src, sig, pos),
        FragKind::Other => scan_until_next(src, sig, pos, next_pat, rep_sep, after_rep),
    }
}

fn match_path(sig: &[Token], mut i: usize) -> Option<usize> {
    let start = i;
    if i + 1 < sig.len() && sig[i].kind == T::Colon && sig[i + 1].kind == T::Colon {
        i += 2;
    }
    loop {
        let t = sig.get(i)?;
        if !(matches!(t.kind, T::Ident | T::RawIdent) || matches!(t.kind, T::Crate | T::Super | T::SelfKw | T::SelfType)) {
            break;
        }
        i += 1;
        if i + 1 < sig.len() && sig[i].kind == T::Colon && sig[i + 1].kind == T::Colon {
            i += 2;
        } else {
            break;
        }
    }
    (i > start).then_some(i)
}

fn match_meta(sig: &[Token], pos: usize) -> Option<usize> {
    let mut i = match_path(sig, pos)?;
    if let Some(t) = sig.get(i) {
        if matching_close(t.kind).is_some() {
            i = take_delimited(sig, i)?.1;
        } else if t.kind == T::Eq {
            let v = sig.get(i + 1)?;
            i = if matching_close(v.kind).is_some() { take_delimited(sig, i + 1)?.1 } else { i + 2 };
        }
    }
    Some(i)
}

fn match_item(src: &str, sig: &[Token], mut i: usize) -> Option<usize> {
    if i >= sig.len() {
        return None;
    }
    while i + 1 < sig.len() && sig[i].kind == T::Hash && sig[i + 1].kind == T::LBracket {
        i = take_delimited(sig, i + 1)?.1;
    }
    if sig.get(i).map(|t| t.kind) == Some(T::Pub) {
        i += 1;
        if sig.get(i).map(|t| t.kind) == Some(T::LParen) {
            i = take_delimited(sig, i)?.1;
        }
    }
    while let Some(t) = sig.get(i) {
        let is_prefix = matches!(t.kind, T::Unsafe | T::Async)
            || (t.kind == T::Const && sig.get(i + 1).is_some_and(|n| matches!(n.kind, T::Fn | T::Unsafe | T::Async | T::Extern | T::Trait | T::Impl)))
            || (t.kind == T::Ident && matches!(t.text(src), "default" | "auto"));
        if is_prefix {
            i += 1;
        } else {
            break;
        }
    }
    let head = sig.get(i)?.kind;
    let is_brace_kind = matches!(head, T::Impl | T::Trait | T::Enum) || (head == T::Ident && sig.get(i).map(|t| t.text(src)) == Some("union"));
    while i < sig.len() {
        match sig[i].kind {
            T::Semi => return Some(i + 1),
            T::LBrace => {
                let (_, after) = take_delimited(sig, i)?;
                return Some(if is_brace_kind || after >= sig.len() || sig[after].kind != T::Semi { after } else { after + 1 });
            }
            T::LParen | T::LBracket => i = take_delimited(sig, i)?.1,
            _ => i += 1,
        }
    }
    None
}

fn pat_head_lit(p: &PatElem) -> Option<(&str, T)> {
    match p {
        PatElem::Lit(t, k) => Some((t.as_str(), *k)),
        PatElem::Rep { inner, .. } => inner.first().and_then(pat_head_lit),
        _ => None,
    }
}

fn scan_until_next(src: &str, sig: &[Token], pos: usize, next_pat: Option<&PatElem>, rep_sep: Option<(&str, T)>, after_rep: Option<&PatElem>) -> Option<usize> {
    let stop1 = next_pat.and_then(pat_head_lit);
    let stop2 = after_rep.and_then(pat_head_lit);
    let mut i = pos;
    let mut turbo_angles = 0i32;
    while i < sig.len() {
        let tk = sig[i].kind;
        if i > pos && turbo_angles == 0 {
            let txt = sig[i].text(src);
            if stop1.is_some_and(|(st, sk)| tk == sk && txt == st) || rep_sep.is_some_and(|(st, sk)| tk == sk && txt == st) || stop2.is_some_and(|(st, sk)| tk == sk && txt == st) {
                return Some(i);
            }
        }
        if tk == T::Lt && (turbo_angles > 0 || (i >= 2 && sig[i - 1].kind == T::Colon && sig[i - 2].kind == T::Colon)) {
            turbo_angles += 1;
            i += 1;
        } else if tk == T::Gt && turbo_angles > 0 && !(i > 0 && matches!(sig[i - 1].kind, T::Minus | T::Eq) && sig[i - 1].is_joint()) {
            turbo_angles -= 1;
            i += 1;
        } else if matching_close(tk).is_some() {
            i = take_delimited(sig, i)?.1;
        } else {
            i += 1;
        }
    }
    (i > pos).then_some(i)
}

fn transcribe_seq(tpl: &[TplElem], binds: &Bindings<'_>, crate_prefix: &str, out: &mut String, spans: &mut Vec<ExpandedSpanMap>) -> bool {
    for elem in tpl {
        if out.len() > MAX_EXPANDED_BYTES {
            return false;
        }
        match elem {
            TplElem::Raw(s) => out.push_str(s),
            TplElem::CrateKw => {
                out.push_str(crate_prefix);
                out.push(' ');
            }
            TplElem::Var(name) => {
                let Some((_, BoundVal::Single(s, orig_s))) = binds.iter().rfind(|(n, _)| n == name) else {
                    return false;
                };
                let es = out.len() as u32;
                out.push_str(s);
                let ee = out.len() as u32;
                if ee > es {
                    spans.push(ExpandedSpanMap { exp_start: es, exp_end: ee, orig_start: *orig_s });
                }
                out.push(' ');
            }
            TplElem::Group { open, close, inner } => {
                out.push(*open);
                if !transcribe_seq(inner, binds, crate_prefix, out, spans) {
                    return false;
                }
                out.push(*close);
                out.push(' ');
            }
            TplElem::Rep { inner, sep, op } => {
                let Some(len) = find_rep_len(inner, binds) else {
                    if *op == RepOp::OneOrMore {
                        return false;
                    }
                    continue;
                };
                for idx in 0..len {
                    if idx > 0 {
                        if let Some(s) = sep {
                            out.push_str(s);
                            out.push(' ');
                        }
                    }
                    let mut sliced = Bindings::with_capacity(binds.len());
                    for (k, v) in binds {
                        match v {
                            BoundVal::Single(s, os) => sliced.push((k.clone(), BoundVal::Single(s, *os))),
                            BoundVal::Seq(items) => {
                                if let Some(iv) = items.get(idx) {
                                    sliced.push((k.clone(), iv.clone()));
                                }
                            }
                        }
                    }
                    if !transcribe_seq(inner, &sliced, crate_prefix, out, spans) {
                        return false;
                    }
                }
            }
        }
    }
    true
}

fn find_rep_len(inner: &[TplElem], binds: &Bindings<'_>) -> Option<usize> {
    for e in inner {
        match e {
            TplElem::Var(name) => {
                if let Some((_, BoundVal::Seq(items))) = binds.iter().rfind(|(n, _)| n == name) {
                    return Some(items.len());
                }
            }
            TplElem::Group { inner: g, .. } | TplElem::Rep { inner: g, .. } => {
                if let Some(n) = find_rep_len(g, binds) {
                    return Some(n);
                }
            }
            _ => {}
        }
    }
    None
}
