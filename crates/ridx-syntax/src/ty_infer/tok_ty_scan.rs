/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Zero-allocation token-slice delimiter & literal helpers shared by [`super::lower_ty`]
//! and [`super::local_defs`] when inspecting `TokenTree`-backed CST fields and `fn(...)` types.

use super::ty::InferTyId;
use crate::name_res::mbe_expand::ExpandedSpanMap;
use crate::name_res::PrimTy;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::{Token, TokenKind as T};
use crate::tree::{NodeId, SyntaxTree};

#[derive(Clone, Copy, Debug)]
pub(super) struct SubLocalEntry {
    len: u8,
    bytes: [u8; 31],
    pub ty: InferTyId,
}

impl SubLocalEntry {
    pub fn new(name: &str, ty: InferTyId) -> Option<Self> {
        let b = name.as_bytes();
        if b.is_empty() || b.len() > 31 {
            return None;
        }
        let mut bytes = [0u8; 31];
        bytes[..b.len()].copy_from_slice(b);
        Some(Self { len: b.len() as u8, bytes, ty })
    }

    #[inline]
    pub fn matches(&self, name: &str) -> bool {
        name.len() == self.len as usize && &self.bytes[..self.len as usize] == name.as_bytes()
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum SubInferEnv<'a> {
    StdSlice { callsite_node: NodeId },
    Expanded { callsite_node: NodeId, callsite_start: u32, callsite_end: u32, spans: &'a [ExpandedSpanMap], root_slot_base: u32, root_slots_len: u32 },
}

impl<'a> SubInferEnv<'a> {
    #[inline]
    pub fn callsite_node(self) -> NodeId {
        match self {
            Self::StdSlice { callsite_node } | Self::Expanded { callsite_node, .. } => callsite_node,
        }
    }
}

pub(super) fn eval_const_uint_expr(st: &SyntaxTree<'_>, expr: NodeId) -> Option<u64> {
    if matches!(st.kind(expr), K::Literal | K::ConstArg) {
        for tok in st.tokens_of(expr) {
            match tok.kind {
                T::True => return Some(1),
                T::False => return Some(0),
                T::Int => return parse_uint_lit(tok.text(st.src)),
                _ => {}
            }
        }
    }
    None
}

pub(super) fn parse_uint_lit(raw: &str) -> Option<u64> {
    let (radix, body) = if let Some(h) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
        (16u64, h)
    } else if let Some(b) = raw.strip_prefix("0b").or_else(|| raw.strip_prefix("0B")) {
        (2u64, b)
    } else if let Some(o) = raw.strip_prefix("0o").or_else(|| raw.strip_prefix("0O")) {
        (8u64, o)
    } else {
        (10u64, raw)
    };
    let trimmed = if radix == 16 {
        body.strip_suffix("usize")
            .or_else(|| body.strip_suffix("isize"))
            .or_else(|| body.strip_suffix("u128"))
            .or_else(|| body.strip_suffix("i128"))
            .or_else(|| body.strip_suffix("u64"))
            .or_else(|| body.strip_suffix("i64"))
            .or_else(|| body.strip_suffix("u32"))
            .or_else(|| body.strip_suffix("i32"))
            .or_else(|| body.strip_suffix("u16"))
            .or_else(|| body.strip_suffix("i16"))
            .or_else(|| body.strip_suffix("u8"))
            .or_else(|| body.strip_suffix("i8"))
            .unwrap_or(body)
    } else {
        body.trim_end_matches(|c: char| c.is_ascii_alphabetic() || c == '_')
    };
    let mut val = 0u64;
    let mut any_digit = false;
    for b in trimmed.bytes() {
        if b == b'_' {
            continue;
        }
        let d = (b as char).to_digit(radix as u32)? as u64;
        val = val.checked_mul(radix)?.checked_add(d)?;
        any_digit = true;
    }
    any_digit.then_some(val)
}

#[inline]
pub(super) fn next_sig_idx(toks: &[Token], mut idx: usize) -> Option<usize> {
    while idx < toks.len() {
        if !toks[idx].is_trivia() {
            return Some(idx);
        }
        idx += 1;
    }
    None
}

#[inline]
pub(super) fn rfind_sig_idx(toks: &[Token]) -> Option<usize> {
    (0..toks.len()).rev().find(|&i| !toks[i].is_trivia())
}

#[inline]
pub(super) fn is_double_colon(toks: &[Token], idx: usize) -> bool {
    toks[idx].kind == T::Colon && toks[idx].is_joint() && toks.get(idx + 1).is_some_and(|t| t.kind == T::Colon)
}

pub(super) fn skip_leading_attrs_vis(toks: &[Token], mut idx: usize) -> Option<usize> {
    loop {
        idx = next_sig_idx(toks, idx)?;
        if toks[idx].kind == T::Hash {
            let br = next_sig_idx(toks, idx + 1).filter(|&i| toks[i].kind == T::LBracket)?;
            let end_b = matching_delim(toks, br)?;
            idx = end_b + 1;
            continue;
        }
        if toks[idx].kind == T::Pub {
            idx += 1;
            if let Some(lp) = next_sig_idx(toks, idx).filter(|&i| toks[i].kind == T::LParen) {
                if let Some(kw) = next_sig_idx(toks, lp + 1) {
                    if matches!(toks[kw].kind, T::Crate | T::SelfKw | T::Super | T::In) {
                        if let Some(rp) = matching_delim(toks, lp) {
                            idx = rp + 1;
                        }
                    }
                }
            }
            continue;
        }
        return Some(idx);
    }
}

pub(super) fn matching_delim(toks: &[Token], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (i, t) in toks[open..].iter().enumerate() {
        match t.kind {
            T::LParen | T::LBrace | T::LBracket => depth += 1,
            T::RParen | T::RBrace | T::RBracket => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i);
                }
            }
            _ => {}
        }
    }
    None
}

pub(super) fn matching_angle(toks: &[Token], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut paren_depth = 0i32;
    let mut i = open;
    while i < toks.len() {
        match toks[i].kind {
            T::LParen | T::LBrace | T::LBracket => paren_depth += 1,
            T::RParen | T::RBrace | T::RBracket => paren_depth -= 1,
            T::Minus if paren_depth == 0 && toks.get(i + 1).is_some_and(|t| t.kind == T::Gt) => {
                i += 2;
                continue;
            }
            T::Lt if paren_depth == 0 => depth += 1,
            T::Gt if paren_depth == 0 => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

pub(super) fn find_top_tok(toks: &[Token], kind: T) -> Option<usize> {
    let (mut d_par, mut d_ang) = (0i32, 0i32);
    for (i, t) in toks.iter().enumerate() {
        match t.kind {
            T::LParen | T::LBrace | T::LBracket => d_par += 1,
            T::RParen | T::RBrace | T::RBracket => d_par -= 1,
            T::Lt if d_par == 0 => d_ang += 1,
            T::Gt if d_par == 0 && d_ang > 0 => d_ang -= 1,
            k if k == kind && d_par == 0 && d_ang == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

pub(super) fn for_each_top_comma(toks: &[Token], mut f: impl FnMut(&[Token])) {
    let mut dummy = false;
    for_each_top_comma_flag(toks, &mut dummy, &mut f);
}

pub(super) fn for_each_top_comma_flag(toks: &[Token], saw_comma: &mut bool, mut f: impl FnMut(&[Token])) {
    let (mut start, mut d_par, mut d_ang) = (0usize, 0i32, 0i32);
    let mut i = 0;
    while i < toks.len() {
        match toks[i].kind {
            T::LParen | T::LBrace | T::LBracket => d_par += 1,
            T::RParen | T::RBrace | T::RBracket => d_par -= 1,
            T::Minus if d_par == 0 && toks.get(i + 1).is_some_and(|t| t.kind == T::Gt) => {
                i += 2;
                continue;
            }
            T::Lt if d_par == 0 => d_ang += 1,
            T::Gt if d_par == 0 && d_ang > 0 => d_ang -= 1,
            T::Comma if d_par == 0 && d_ang == 0 => {
                *saw_comma = true;
                f(&toks[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < toks.len() {
        f(&toks[start..]);
    }
}

pub(super) fn parse_int_or_float_suffix(text: &str) -> Option<InferTyId> {
    const INT_SUFFIXES: [(&str, PrimTy); 12] = [
        ("i128", PrimTy::I128),
        ("u128", PrimTy::U128),
        ("isize", PrimTy::Isize),
        ("usize", PrimTy::Usize),
        ("i64", PrimTy::I64),
        ("u64", PrimTy::U64),
        ("i32", PrimTy::I32),
        ("u32", PrimTy::U32),
        ("i16", PrimTy::I16),
        ("u16", PrimTy::U16),
        ("i8", PrimTy::I8),
        ("u8", PrimTy::U8),
    ];
    for (suf, prim) in INT_SUFFIXES {
        if text.ends_with(suf) {
            return Some(InferTyId::from_prim(prim));
        }
    }
    if !text.starts_with("0x") && !text.starts_with("0X") {
        return parse_float_suffix(text);
    }
    None
}

pub(super) fn parse_float_suffix(text: &str) -> Option<InferTyId> {
    if text.ends_with("f32") {
        Some(InferTyId::F32)
    } else if text.ends_with("f64") {
        Some(InferTyId::F64)
    } else {
        None
    }
}

pub(super) fn decoded_byte_str_len(text: &str, is_raw: bool) -> u64 {
    if is_raw {
        let after_br = text.strip_prefix("br").unwrap_or(text);
        let hashes = after_br.bytes().take_while(|&b| b == b'#').count();
        let prefix = 1 + hashes;
        return after_br.len().saturating_sub(prefix * 2) as u64;
    }
    let inner = text.strip_prefix("b\"").and_then(|s| s.strip_suffix('"')).unwrap_or("");
    let bytes = inner.as_bytes();
    let (mut i, mut count) = (0usize, 0u64);
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'x' => i += 4,
                b'\n' => {
                    i += 2;
                    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                        i += 1;
                    }
                    continue;
                }
                _ => i += 2,
            }
        } else {
            i += 1;
        }
        count += 1;
    }
    count
}
