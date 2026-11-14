/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Functions: the signature (parameters with `self`, return type, where clause) and the body.

use super::items::{self, attrs};
use super::{name, stmts, types, Parser};
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;
use crate::tree::{Event, SyntaxTree};

/// Parses `src`, keeping full CST inside the function containing `target_offset` and flat `BlockExpr`s elsewhere.
pub fn parse_for_offset(src: &str, target_offset: u32) -> (SyntaxTree<'_>, Option<(u32, u32)>) {
    parse_opt(src, Some(target_offset))
}

pub(crate) fn parse_opt(src: &str, target_offset: Option<u32>) -> (SyntaxTree<'_>, Option<(u32, u32)>) {
    let tokens = match target_offset {
        Some(off) => crate::lexer::lex_for_offset(src, off),
        None => crate::lex(src),
    };
    let mut sig: Vec<u32> = Vec::with_capacity(tokens.len() * 3 / 4 + 8);
    let mut i = 0usize;
    let tlen = tokens.len();
    while i < tlen {
        if !tokens[i].is_trivia() {
            sig.push(i as u32);
        }
        i += 1;
    }
    let events_cap = if target_offset.is_some() { sig.len() + 512 } else { sig.len() * 3 + 16 };
    let mut p = Parser {
        src,
        tokens: &tokens,
        sig: &sig,
        pos: 0,
        events: Vec::with_capacity(events_cap),
        nodes_count: 0,
        target_offset,
        focused_span: None,
        skipped_fn: false,
        #[cfg(debug_assertions)]
        fuel: 0,
    };
    let m = p.start();
    items::item_list(&mut p, false);
    while !p.at_eof() {
        p.err_bump();
    }
    m.complete(&mut p, K::SourceFile);
    let Parser { events, nodes_count, focused_span, .. } = p;
    let span = target_offset.map(|o| focused_span.unwrap_or((o, o)));
    (SyntaxTree::build(src, tokens, &sig, events, nodes_count as usize), span)
}

pub(crate) fn try_skip_non_target_item(p: &mut Parser<'_>) -> bool {
    let Some(target) = p.target_offset else { return false };
    let len = p.sig.len();
    if p.pos >= len {
        return false;
    }
    let item_start = p.tokens[p.sig[p.pos] as usize].start;
    let mut idx = p.pos;
    while idx + 2 < len && p.tokens[p.sig[idx] as usize].kind == T::Hash && p.tokens[p.sig[idx + 1] as usize].kind == T::LBracket {
        idx += 2;
        let mut b = 1u32;
        while idx < len && b > 0 {
            match p.tokens[p.sig[idx] as usize].kind {
                T::LBracket => b += 1,
                T::RBracket => b -= 1,
                _ => {}
            }
            idx += 1;
        }
    }
    if idx < len && p.tokens[p.sig[idx] as usize].kind == T::Pub {
        idx += 1;
        if idx < len && p.tokens[p.sig[idx] as usize].kind == T::LParen {
            idx += 1;
            let mut d = 1u32;
            while idx < len && d > 0 {
                match p.tokens[p.sig[idx] as usize].kind {
                    T::LParen => d += 1,
                    T::RParen => d -= 1,
                    _ => {}
                }
                idx += 1;
            }
        }
    }
    while idx < len {
        let tk = p.tokens[p.sig[idx] as usize];
        let nk = (idx + 1 < len).then(|| p.tokens[p.sig[idx + 1] as usize].kind);
        match tk.kind {
            T::Unsafe | T::Async if nk != Some(T::LBrace) => idx += 1,
            T::Const if matches!(nk, Some(T::Fn | T::Unsafe | T::Async | T::Extern | T::Trait | T::Impl)) => idx += 1,
            T::Extern if nk == Some(T::Fn) => idx += 1,
            T::Extern if nk.is_some_and(|k| k.is_literal()) && idx + 2 < len && p.tokens[p.sig[idx + 2] as usize].kind == T::Fn => idx += 2,
            T::Ident
                if matches!(nk, Some(T::Fn | T::Impl | T::Unsafe | T::Async | T::Const | T::Trait | T::Extern | T::Static))
                    && matches!(&p.src[tk.start as usize..tk.end as usize], "default" | "safe" | "auto") =>
            {
                idx += 1;
            }
            _ => break,
        }
    }
    if idx >= len {
        return false;
    }
    let (kind, need_params, allow_semi, allow_brace) = match p.tokens[p.sig[idx] as usize].kind {
        T::Fn => (K::Fn, true, true, true),
        T::Impl => (K::Impl, false, false, true),
        T::Trait => (K::Trait, false, true, true),
        T::Struct => (K::Struct, false, true, true),
        T::Enum => (K::Enum, false, false, true),
        T::Const => (K::Const, false, true, false),
        T::Static => (K::Static, false, true, false),
        T::Use => (K::Use, false, true, false),
        T::Type => (K::TypeAlias, false, true, false),
        _ => return false,
    };
    let mut i = idx + 1;
    let (mut p_depth, mut b_depth, mut c_depth) = (0u32, 0u32, 0u32);
    let mut saw_params = !need_params
        || (i < len && p.tokens[p.sig[i] as usize].kind == T::Semi)
        || (i + 1 < len && p.tokens[p.sig[i] as usize].kind == T::LBrace && p.tokens[p.sig[i + 1] as usize].kind == T::RBrace);
    let mut end_idx = None;
    while i < len {
        match p.tokens[p.sig[i] as usize].kind {
            T::LParen => {
                if c_depth == 0 {
                    saw_params = true;
                }
                p_depth += 1;
            }
            T::RParen => p_depth = p_depth.saturating_sub(1),
            T::LBracket => b_depth += 1,
            T::RBracket => b_depth = b_depth.saturating_sub(1),
            T::Semi if saw_params && p_depth == 0 && b_depth == 0 && c_depth == 0 => {
                if allow_semi {
                    end_idx = Some(i);
                }
                break;
            }
            T::LBrace => {
                if allow_brace && saw_params && p_depth == 0 && b_depth == 0 && c_depth == 0 && p.tokens[p.sig[i - 1] as usize].kind != T::Eq {
                    let mut d = 1u32;
                    let mut j = i + 1;
                    while j < len && d > 0 {
                        match p.tokens[p.sig[j] as usize].kind {
                            T::LBrace => d += 1,
                            T::RBrace => d -= 1,
                            _ => {}
                        }
                        j += 1;
                    }
                    if d == 0 {
                        end_idx = Some(j - 1);
                    }
                    break;
                }
                c_depth += 1;
            }
            T::RBrace => {
                if c_depth == 0 && p_depth == 0 && b_depth == 0 {
                    break;
                }
                c_depth = c_depth.saturating_sub(1);
            }
            _ => {}
        }
        i += 1;
    }
    let Some(close) = end_idx else { return false };
    let close_end = p.tokens[p.sig[close] as usize].end;
    if item_start <= target && target <= close_end {
        if matches!(kind, K::Fn | K::Const | K::Static | K::Struct | K::Enum) && p.focused_span.is_none_or(|(s, e)| close_end.saturating_sub(item_start) < e.saturating_sub(s)) {
            p.focused_span = Some((item_start, close_end));
        }
        return false;
    }
    p.skipped_fn = true;
    let m = p.start();
    let count = close + 1 - p.pos;
    p.pos = close + 1;
    #[cfg(debug_assertions)]
    {
        p.fuel = 0;
    }
    p.events.push(Event::tokens(count as u32));
    m.complete(p, kind);
    true
}

pub(crate) fn try_skip_mod_item_list(p: &mut Parser<'_>, mod_start: u32) -> bool {
    let Some(target) = p.target_offset else { return false };
    if p.nth_at(1, T::Hash) && p.nth_at(2, T::Bang) {
        return false;
    }
    let len = p.sig.len();
    let mut idx = p.pos;
    let mut depth = 0u32;
    let mut close_pos = None;
    while idx < len {
        let tk = p.tokens[p.sig[idx] as usize];
        match tk.kind {
            T::Mod => return false,
            T::Ident if tk.end - tk.start == 11 && &p.src[tk.start as usize..tk.end as usize] == "macro_rules" => return false,
            T::LBrace => depth += 1,
            T::RBrace => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    close_pos = Some(idx);
                    break;
                }
            }
            _ => {}
        }
        idx += 1;
    }
    let Some(close) = close_pos else { return false };
    let close_end = p.tokens[p.sig[close] as usize].end;
    if mod_start <= target && target <= close_end {
        return false;
    }
    p.skipped_fn = true;
    let m = p.start();
    let count = close + 1 - p.pos;
    p.pos = close + 1;
    #[cfg(debug_assertions)]
    {
        p.fuel = 0;
    }
    p.events.push(Event::tokens(count as u32));
    m.complete(p, K::ItemList);
    true
}

pub(crate) fn fn_(p: &mut Parser<'_>) -> K {
    let fn_start = p.tok(0).map_or(0, |t| t.start);
    p.bump();
    name(p);
    types::opt_generic_params(p);
    if p.at(T::LParen) {
        param_list(p);
    } else {
        p.expect(T::LParen);
    }
    if p.at_op("->") {
        // A function's return type takes bounds: `-> impl Iterator<Item = u8> + 'a`.
        let r = p.start();
        p.bump_n(2);
        types::type_(p);
        r.complete(p, K::RetType);
    }
    types::opt_where_clause(p);
    if p.at(T::LBrace) {
        if !try_skip_brace_node(p, fn_start, K::BlockExpr) {
            stmts::block(p);
        }
    } else {
        p.expect(T::Semi);
    }
    K::Fn
}

pub(crate) fn try_skip_brace_node(p: &mut Parser<'_>, item_start: u32, kind: K) -> bool {
    let Some(target) = p.target_offset else { return false };
    let mut depth = 0u32;
    let mut idx = p.pos;
    let len = p.sig.len();
    let mut close_pos = None;
    while idx < len {
        match p.tokens[p.sig[idx] as usize].kind {
            T::LBrace => depth += 1,
            T::RBrace => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    close_pos = Some(idx);
                    break;
                }
            }
            _ => {}
        }
        idx += 1;
    }
    let Some(close) = close_pos else { return false };
    let close_end = p.tokens[p.sig[close] as usize].end;
    if item_start <= target && target <= close_end {
        if p.focused_span.is_none_or(|(s, e)| close_end.saturating_sub(item_start) < e.saturating_sub(s)) {
            p.focused_span = Some((item_start, close_end));
        }
        return false;
    }
    p.skipped_fn = true;
    let m = p.start();
    let count = close + 1 - p.pos;
    p.pos = close + 1;
    #[cfg(debug_assertions)]
    {
        p.fuel = 0;
    }
    p.events.push(Event::tokens(count as u32));
    m.complete(p, kind);
    true
}

pub(crate) fn opt_ret_type(p: &mut Parser<'_>) {
    if p.at_op("->") {
        let m = p.start();
        p.bump_n(2);
        types::type_no_bounds(p);
        m.complete(p, K::RetType);
    }
}

/// `(self, a: T, ..)` of a function.
fn param_list(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump();
    let mut first = true;
    while !p.at_eof() && !p.at(T::RParen) {
        let before = p.mark_pos();
        let pm = p.start();
        attrs(p);
        if first && at_self_param(p) {
            self_param(p);
            pm.complete(p, K::SelfParam);
        } else if p.at_op("...") {
            p.bump_n(3);
            pm.complete(p, K::Param);
        } else {
            super::patterns::pattern_no_top_alt(p);
            if p.eat(T::Colon) {
                if p.at_op("...") {
                    p.bump_n(3);
                } else {
                    types::type_(p);
                }
            }
            pm.complete(p, K::Param);
        }
        first = false;
        if !p.eat(T::Comma) && !p.at(T::RParen) {
            if p.mark_pos() == before {
                p.err_bump();
            } else {
                break;
            }
        }
    }
    p.expect(T::RParen);
    m.complete(p, K::ParamList);
}

fn at_self_param(p: &mut Parser<'_>) -> bool {
    let mut i = 0;
    if p.nth_at(0, T::Amp) {
        i = 1;
        if p.nth_at(i, T::Lifetime) {
            i += 1;
        }
    }
    if p.nth_at(i, T::Mut) {
        i += 1;
    }
    // `self`, not the path `self::x` of a pattern.
    p.nth_at(i, T::SelfKw) && !p.nth_at_op(i + 1, "::")
}

fn self_param(p: &mut Parser<'_>) {
    if p.eat(T::Amp) && p.at(T::Lifetime) {
        let l = p.start();
        p.bump();
        l.complete(p, K::Lifetime);
    }
    p.eat(T::Mut);
    p.bump();
    if p.eat(T::Colon) {
        types::type_(p);
    }
}
