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

use super::items::attrs;
use super::{name, stmts, types, Parser};
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;

pub(crate) fn fn_(p: &mut Parser<'_>) -> K {
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
        stmts::block(p);
    } else {
        p.expect(T::Semi);
    }
    K::Fn
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
