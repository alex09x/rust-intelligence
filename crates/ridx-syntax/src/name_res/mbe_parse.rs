/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Declarative `macro_rules!` arm parser (`(pattern) => { template }; ...`) into compact ASTs
//! consumed by [`super::mbe_expand`].

use crate::lexer::lex;
use crate::sema_fn::norm_ident;
use crate::token::{Token, TokenKind as T};

/// Parsed declarative `macro_rules!` definition (`(pattern) => { template }; ...`).
#[derive(Clone, Debug)]
pub struct MacroRulesDef {
    pub(super) arms: Vec<(Vec<PatElem>, Vec<TplElem>)>,
}

#[derive(Clone, Debug)]
pub(super) enum PatElem {
    Lit(String, T),
    Group { open: T, inner: Vec<PatElem> },
    Var { name: String, frag: FragKind },
    Rep { inner: Vec<PatElem>, sep: Option<(String, T, Option<T>)>, op: RepOp },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FragKind {
    Tt,
    Ident,
    Meta,
    Vis,
    Item,
    Block,
    Literal,
    Path,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RepOp {
    ZeroOrMore,
    OneOrMore,
    ZeroOrOne,
}

#[derive(Clone, Debug)]
pub(super) enum TplElem {
    Raw(String),
    CrateKw,
    Var(String),
    Group { open: char, close: char, inner: Vec<TplElem> },
    Rep { inner: Vec<TplElem>, sep: Option<String>, op: RepOp },
}

impl MacroRulesDef {
    /// Parses a `macro_rules!` body (`{ (pat) => { body }; ... }` or bare `(pat) => { body }; ...`).
    pub fn parse_body(raw_body: &str) -> Option<Self> {
        let toks = lex(raw_body);
        let sig: Vec<Token> = toks.into_iter().filter(|t| !t.is_trivia()).collect();
        let slice = strip_outer_group(&sig);
        let mut arms = Vec::new();
        let mut i = 0;
        while i < slice.len() {
            if slice[i].kind == T::Semi {
                i += 1;
                continue;
            }
            let (pat_inner, next_i) = take_delimited(slice, i)?;
            i = next_i;
            if i + 1 >= slice.len() || slice[i].kind != T::Eq || slice[i + 1].kind != T::Gt {
                return None;
            }
            i += 2;
            let (tpl_inner, next_i2) = take_delimited(slice, i)?;
            i = next_i2;
            if i < slice.len() && slice[i].kind == T::Semi {
                i += 1;
            }
            let pat = parse_pat(raw_body, pat_inner)?;
            let tpl = parse_tpl(raw_body, tpl_inner)?;
            arms.push((pat, tpl));
        }
        (!arms.is_empty()).then_some(Self { arms })
    }
}

pub(super) fn strip_outer_group(sig: &[Token]) -> &[Token] {
    if sig.len() >= 2 && matches!(sig[0].kind, T::LBrace | T::LParen | T::LBracket) {
        if let Some((inner, end)) = take_delimited(sig, 0) {
            if end == sig.len() {
                return inner;
            }
        }
    }
    sig
}

pub(super) fn take_delimited(sig: &[Token], start: usize) -> Option<(&[Token], usize)> {
    let close = matching_close(sig.get(start)?.kind)?;
    let mut depth = 0u32;
    for (off, t) in sig[start..].iter().enumerate() {
        if t.kind == sig[start].kind {
            depth += 1;
        } else if t.kind == close {
            depth -= 1;
            if depth == 0 {
                return Some((&sig[start + 1..start + off], start + off + 1));
            }
        }
    }
    None
}

#[inline]
pub(super) fn matching_close(k: T) -> Option<T> {
    match k {
        T::LParen => Some(T::RParen),
        T::LBracket => Some(T::RBracket),
        T::LBrace => Some(T::RBrace),
        _ => None,
    }
}

fn parse_pat(src: &str, sig: &[Token]) -> Option<Vec<PatElem>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < sig.len() {
        if sig[i].kind == T::Dollar {
            let next = sig.get(i + 1)?;
            if next.kind == T::LParen {
                let (rep_inner, mut after) = take_delimited(sig, i + 1)?;
                let mut sep = None;
                if after < sig.len() && !matches!(sig[after].kind, T::Star | T::Plus | T::Question) {
                    let (stxt, sk1) = (sig[after].text(src).to_string(), sig[after].kind);
                    let joint1 = sig[after].is_joint();
                    after += 1;
                    let sk2 = if joint1 && after < sig.len() && !matches!(sig[after].kind, T::Star | T::Plus | T::Question) {
                        after += 1;
                        Some(sig[after - 1].kind)
                    } else {
                        None
                    };
                    sep = Some((stxt, sk1, sk2));
                }
                let op = match sig.get(after)?.kind {
                    T::Star => RepOp::ZeroOrMore,
                    T::Plus => RepOp::OneOrMore,
                    T::Question => RepOp::ZeroOrOne,
                    _ => return None,
                };
                out.push(PatElem::Rep { inner: parse_pat(src, rep_inner)?, sep, op });
                i = after + 1;
                continue;
            }
            if (matches!(next.kind, T::Ident | T::RawIdent) || next.kind.is_keyword()) && sig.get(i + 2).map(|t| t.kind) == Some(T::Colon) {
                let vname = norm_ident(next.text(src)).to_string();
                let frag = match sig.get(i + 3)?.text(src) {
                    "tt" => FragKind::Tt,
                    "ident" => FragKind::Ident,
                    "meta" => FragKind::Meta,
                    "vis" => FragKind::Vis,
                    "item" => FragKind::Item,
                    "block" => FragKind::Block,
                    "literal" => FragKind::Literal,
                    "path" => FragKind::Path,
                    _ => FragKind::Other,
                };
                out.push(PatElem::Var { name: vname, frag });
                i += 4;
                continue;
            }
        }
        if matching_close(sig[i].kind).is_some() {
            let open = sig[i].kind;
            let (inner, after) = take_delimited(sig, i)?;
            out.push(PatElem::Group { open, inner: parse_pat(src, inner)? });
            i = after;
        } else {
            out.push(PatElem::Lit(sig[i].text(src).to_string(), sig[i].kind));
            i += 1;
        }
    }
    Some(out)
}

fn parse_tpl(src: &str, sig: &[Token]) -> Option<Vec<TplElem>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < sig.len() {
        if sig[i].kind == T::Dollar {
            if let Some(next) = sig.get(i + 1) {
                if next.kind == T::Crate {
                    out.push(TplElem::CrateKw);
                    i += 2;
                    continue;
                }
                if next.kind == T::LParen {
                    let (rep_inner, mut after) = take_delimited(sig, i + 1)?;
                    let mut sep = None;
                    if after < sig.len() && !matches!(sig[after].kind, T::Star | T::Plus | T::Question) {
                        let mut stxt = sig[after].text(src).to_string();
                        let joint1 = sig[after].is_joint();
                        after += 1;
                        if joint1 && after < sig.len() && !matches!(sig[after].kind, T::Star | T::Plus | T::Question) {
                            stxt.push_str(sig[after].text(src));
                            after += 1;
                        }
                        sep = Some(stxt);
                    }
                    let op = match sig.get(after)?.kind {
                        T::Star => RepOp::ZeroOrMore,
                        T::Plus => RepOp::OneOrMore,
                        T::Question => RepOp::ZeroOrOne,
                        _ => return None,
                    };
                    out.push(TplElem::Rep { inner: parse_tpl(src, rep_inner)?, sep, op });
                    i = after + 1;
                    continue;
                }
                if matches!(next.kind, T::Ident | T::RawIdent) || next.kind.is_keyword() {
                    out.push(TplElem::Var(norm_ident(next.text(src)).to_string()));
                    i += 2;
                    continue;
                }
            }
        }
        if matching_close(sig[i].kind).is_some() {
            let (open_ch, close_ch) = match sig[i].kind {
                T::LParen => ('(', ')'),
                T::LBracket => ('[', ']'),
                _ => ('{', '}'),
            };
            let (inner, after) = take_delimited(sig, i)?;
            out.push(TplElem::Group { open: open_ch, close: close_ch, inner: parse_tpl(src, inner)? });
            i = after;
        } else {
            let txt = sig[i].text(src);
            let joint = sig[i].is_joint() && i + 1 < sig.len() && !matches!(sig[i + 1].kind, T::Dollar);
            let mut s = String::with_capacity(txt.len() + 1);
            s.push_str(txt);
            if !joint {
                s.push(' ');
            }
            out.push(TplElem::Raw(s));
            i += 1;
        }
    }
    Some(out)
}

pub(super) fn pat_head_lit(p: &PatElem) -> Option<(&str, T, Option<T>)> {
    match p {
        PatElem::Lit(t, k) => Some((t.as_str(), *k, None)),
        PatElem::Rep { inner, .. } => inner.first().and_then(pat_head_lit),
        // A block (`-> $ret:ty $body:block`) or a `{ … }` group opens with the first top-level `{`.
        PatElem::Var { frag: FragKind::Block, .. } | PatElem::Group { open: T::LBrace, .. } => Some(("{", T::LBrace, None)),
        _ => None,
    }
}

pub(super) fn matches_stop(src: &str, sig: &[Token], i: usize, (st, sk, sk2): (&str, T, Option<T>)) -> bool {
    if sig[i].kind != sk || sig[i].text(src) != st {
        return false;
    }
    if let Some(k2) = sk2 {
        return sig.get(i + 1).map(|t| t.kind) == Some(k2);
    }
    if sk == T::Colon && ((sig[i].is_joint() && sig.get(i + 1).map(|t| t.kind) == Some(T::Colon)) || (i > 0 && sig[i - 1].kind == T::Colon && sig[i - 1].is_joint())) {
        return false;
    }
    if sk == T::Eq
        && ((sig[i].is_joint() && sig.get(i + 1).map(|t| t.kind) == Some(T::Eq)) || (i > 0 && matches!(sig[i - 1].kind, T::Eq | T::Bang | T::Lt | T::Gt) && sig[i - 1].is_joint()))
    {
        return false;
    }
    true
}

pub(super) fn scan_until_next(
    src: &str,
    sig: &[Token],
    pos: usize,
    next_pat: Option<&PatElem>,
    rep_sep: Option<(&str, T, Option<T>)>,
    after_rep: Option<&PatElem>,
) -> Option<usize> {
    let (stop1, stop2) = (next_pat.and_then(pat_head_lit), after_rep.and_then(pat_head_lit));
    let mut i = pos;
    let mut turbo_angles = 0i32;
    while i < sig.len() {
        let tk = sig[i].kind;
        if i > pos
            && turbo_angles == 0
            && (stop1.is_some_and(|s| matches_stop(src, sig, i, s)) || rep_sep.is_some_and(|s| matches_stop(src, sig, i, s)) || stop2.is_some_and(|s| matches_stop(src, sig, i, s)))
        {
            return Some(i);
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

#[cfg(test)]
mod tests {
    use super::MacroRulesDef;

    #[test]
    fn a_type_ends_where_the_block_after_it_opens() {
        let def = MacroRulesDef::parse_body("{ (fn $name:ident() -> $ret:ty $body:block) => { pub fn $name() -> $ret $body }; }").unwrap();
        let out = def.expand("fn answer() -> Option<u8> { Some(42) }", "crate").unwrap();
        assert_eq!(out.split_whitespace().collect::<String>(), "pubfnanswer()->Option<u8>{Some(42)}");
    }
}
