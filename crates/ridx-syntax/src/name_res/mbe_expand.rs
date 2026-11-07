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

#[derive(Clone, Debug)]
enum BoundVal<'s> {
    Single(&'s str),
    Seq(Vec<BoundVal<'s>>),
}

type Bindings<'s> = Vec<(String, BoundVal<'s>)>;

impl MacroRulesDef {
    /// Attempts to expand `call_inner_src` (the inner text of `foo! { ... }` / `foo!(...)`).
    pub fn expand(&self, call_inner_src: &str, crate_prefix: &str) -> Option<String> {
        let toks = lex(call_inner_src);
        let sig: Vec<Token> = toks.into_iter().filter(|t| !t.is_trivia()).collect();
        for (pat, tpl) in &self.arms {
            let mut binds = Bindings::new();
            if match_seq(call_inner_src, &sig, 0, pat, &mut binds) == Some(sig.len()) {
                let mut out = String::new();
                if transcribe_seq(tpl, &binds, crate_prefix, &mut out) && out.len() <= MAX_EXPANDED_BYTES {
                    return Some(out);
                }
            }
        }
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

fn match_seq<'s>(src: &'s str, sig: &[Token], mut pos: usize, pats: &[PatElem], binds: &mut Bindings<'s>) -> Option<usize> {
    for (pi, pat) in pats.iter().enumerate() {
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
                if match_seq(src, grp_inner, 0, inner, binds) != Some(grp_inner.len()) {
                    return None;
                }
                pos = after;
            }
            PatElem::Var { name, frag } => {
                let end = match_frag(src, sig, pos, *frag, pats.get(pi + 1))?;
                let slice_str = if pos == end { "" } else { &src[sig[pos].start as usize..sig[end - 1].end as usize] };
                binds.push((name.clone(), BoundVal::Single(slice_str)));
                pos = end;
            }
            PatElem::Rep { inner, sep, op } => {
                let mut vars = Vec::new();
                collect_pat_vars(inner, &mut vars);
                let mut per_var: Vec<Vec<BoundVal<'s>>> = vec![Vec::new(); vars.len()];
                let mut count = 0usize;
                loop {
                    if *op == RepOp::ZeroOrOne && count == 1 {
                        break;
                    }
                    let mut iter_pos = pos;
                    if count > 0 {
                        if let Some((stxt, skind)) = sep {
                            let Some(t) = sig.get(iter_pos) else { break };
                            if t.kind != *skind || t.text(src) != stxt {
                                break;
                            }
                            iter_pos += 1;
                        }
                    }
                    let mut iter_binds = Bindings::new();
                    let Some(next_pos) = match_seq(src, sig, iter_pos, inner, &mut iter_binds) else {
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

fn match_frag(src: &str, sig: &[Token], pos: usize, frag: FragKind, next_pat: Option<&PatElem>) -> Option<usize> {
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
            } else if matches!(tok.kind, T::RParen | T::RBracket | T::RBrace) {
                None
            } else {
                Some(pos + 1)
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
        FragKind::Other => scan_until_next(src, sig, pos, next_pat),
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

fn scan_until_next(src: &str, sig: &[Token], pos: usize, next_pat: Option<&PatElem>) -> Option<usize> {
    let mut i = pos;
    while i < sig.len() {
        if i > pos {
            if let Some(PatElem::Lit(txt, kind)) = next_pat {
                if sig[i].kind == *kind && sig[i].text(src) == txt {
                    return Some(i);
                }
            }
        }
        i = if matching_close(sig[i].kind).is_some() { take_delimited(sig, i)?.1 } else { i + 1 };
    }
    (i > pos).then_some(i)
}

fn transcribe_seq(tpl: &[TplElem], binds: &Bindings<'_>, crate_prefix: &str, out: &mut String) -> bool {
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
                let Some((_, BoundVal::Single(s))) = binds.iter().rfind(|(n, _)| n == name) else {
                    return false;
                };
                out.push_str(s);
                out.push(' ');
            }
            TplElem::Group { open, close, inner } => {
                out.push(*open);
                if !transcribe_seq(inner, binds, crate_prefix, out) {
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
                            BoundVal::Single(s) => sliced.push((k.clone(), BoundVal::Single(s))),
                            BoundVal::Seq(items) => {
                                if let Some(iv) = items.get(idx) {
                                    sliced.push((k.clone(), iv.clone()));
                                }
                            }
                        }
                    }
                    if !transcribe_seq(inner, &sliced, crate_prefix, out) {
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
