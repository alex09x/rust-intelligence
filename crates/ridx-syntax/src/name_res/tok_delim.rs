/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Pure non-trivia token navigation and balanced-delimiter helpers for [`super::tok_slice_scan`],
//! [`super::adt_tok_walk`], and [`super::macro_tt_walk`].

use crate::token::{Token, TokenKind as T};

#[inline]
pub(super) fn next_sig(toks: &[Token], mut idx: usize) -> Option<(usize, Token)> {
    while idx < toks.len() {
        let t = toks[idx];
        if !t.is_trivia() {
            return Some((idx, t));
        }
        idx += 1;
    }
    None
}

#[inline]
pub(super) fn next_sig_matching(toks: &[Token], mut idx: usize, want: T) -> Option<(usize, Token)> {
    while let Some((i, t)) = next_sig(toks, idx) {
        if t.kind == want {
            return Some((i, t));
        }
        idx = i + 1;
    }
    None
}

#[inline]
pub(super) fn rfind_sig(toks: &[Token]) -> Option<usize> {
    toks.iter().rposition(|t| !t.is_trivia())
}

#[inline]
pub(super) fn is_colon2(toks: &[Token], idx: usize) -> bool {
    toks[idx].kind == T::Colon && toks[idx].is_joint() && toks.get(idx + 1).is_some_and(|t| t.kind == T::Colon)
}

#[inline]
pub(super) fn colon2_after(_toks: &[Token], idx: usize) -> usize {
    idx + 2
}

pub(super) fn skip_vis(toks: &[Token], idx: usize) -> usize {
    let Some((pi, pt)) = next_sig(toks, idx) else {
        return idx;
    };
    if pt.kind != T::Pub {
        return idx;
    }
    if let Some((li, lt)) = next_sig(toks, pi + 1) {
        if lt.kind == T::LParen {
            if let Some(close) = find_matching_delim(toks, li) {
                return close + 1;
            }
        }
    }
    pi + 1
}

pub(super) fn find_matching_delim(toks: &[Token], open_idx: usize) -> Option<usize> {
    let (open_k, close_k) = match toks[open_idx].kind {
        T::LParen => (T::LParen, T::RParen),
        T::LBracket => (T::LBracket, T::RBracket),
        T::LBrace => (T::LBrace, T::RBrace),
        _ => return None,
    };
    let mut d = 0u32;
    for (off, t) in toks[open_idx..].iter().enumerate() {
        if t.kind == open_k {
            d += 1;
        } else if t.kind == close_k {
            d -= 1;
            if d == 0 {
                return Some(open_idx + off);
            }
        }
    }
    None
}

pub(super) fn find_matching_angle(toks: &[Token], open_idx: usize) -> Option<usize> {
    let (mut d, mut i) = (0i32, open_idx);
    while i < toks.len() {
        match toks[i].kind {
            T::Minus if toks[i].is_joint() && toks.get(i + 1).map(|t| t.kind) == Some(T::Gt) => i += 1,
            T::Lt => d += 1,
            T::Gt => {
                d -= 1;
                if d == 0 {
                    return Some(i);
                }
            }
            T::Semi | T::LBrace | T::RBrace => return None,
            _ => {}
        }
        i += 1;
    }
    None
}

pub(super) fn find_top_level_tok(toks: &[Token], want: T) -> Option<usize> {
    let (mut d, mut i) = (0i32, 0usize);
    while i < toks.len() {
        match toks[i].kind {
            T::Minus if toks[i].is_joint() && toks.get(i + 1).map(|t| t.kind) == Some(T::Gt) => i += 1,
            T::LParen | T::LBracket | T::LBrace | T::Lt => d += 1,
            T::RParen | T::RBracket | T::RBrace | T::Gt => d = (d - 1).max(0),
            k if k == want && d == 0 => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}
