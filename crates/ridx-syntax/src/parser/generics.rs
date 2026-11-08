/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Generic arguments and parameters, where clauses, `for<..>` binders and bounds.

use super::{
    name, token_tree,
    types::{path, type_, PathMode},
    Parser,
};
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;

/// `<T, 'a, N, Item = U, Item: Bound>`.
pub(crate) fn generic_args(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump();
    while !p.at_eof() && !p.at(T::Gt) {
        let before = p.mark_pos();
        let a = p.start();
        let kind = match p.current() {
            Some(T::Lifetime) => {
                lifetime(p);
                K::LifetimeArg
            }
            Some(T::LBrace) => {
                super::stmts::block(p);
                K::ConstArg
            }
            Some(k) if k.is_literal() || k == T::Minus || k == T::True || k == T::False => {
                if p.eat(T::Minus) {
                    if p.current().is_some_and(|c| c.is_literal() || matches!(c, T::True | T::False)) {
                        p.bump();
                    }
                } else {
                    p.bump();
                }
                K::ConstArg
            }
            Some(T::Ident) if assoc_arg_ahead(p) => {
                name(p);
                if p.at(T::Lt) {
                    // A generic associated type: `Item<'a> = &'a T`.
                    generic_args(p);
                }
                if p.eat(T::Colon) {
                    bounds(p);
                } else {
                    p.bump();
                    type_(p);
                }
                K::AssocTypeArg
            }
            _ => {
                type_(p);
                K::TypeArg
            }
        };
        a.complete(p, kind);
        if !p.eat(T::Comma) {
            if p.mark_pos() == before {
                p.err_bump();
            } else {
                break;
            }
        }
    }
    p.expect(T::Gt);
    m.complete(p, K::GenericArgList);
}

/// Whether an associated item constraint starts here: `Name =`, `Name:`, `Name<..> =`, `Name<..>:`.
fn assoc_arg_ahead(p: &mut Parser<'_>) -> bool {
    let mut i = 1;
    if p.nth_at(1, T::Lt) {
        let mut depth = 0i32;
        loop {
            match p.nth(i) {
                None => return false,
                // The `>` of `->` closes nothing.
                Some(T::Minus) if p.nth_at(i + 1, T::Gt) => i += 1,
                Some(T::Lt) => depth += 1,
                Some(T::Gt) => {
                    depth -= 1;
                    if depth == 0 {
                        i += 1;
                        break;
                    }
                }
                Some(T::LBrace | T::RBrace | T::Semi) => return false,
                _ => {}
            }
            i += 1;
        }
    }
    (p.nth_at(i, T::Eq) && !p.nth_at_op(i, "==")) || (p.nth_at(i, T::Colon) && !p.nth_at_op(i, "::"))
}

pub(crate) fn lifetime(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump();
    m.complete(p, K::Lifetime);
}

/// `<'a, T: Bound = Default, const N: usize>` after an item name.
pub(crate) fn opt_generic_params(p: &mut Parser<'_>) {
    if !p.at(T::Lt) {
        return;
    }
    let m = p.start();
    p.bump();
    while !p.at_eof() && !p.at(T::Gt) {
        let before = p.mark_pos();
        let g = p.start();
        super::items::attrs(p);
        let kind = match p.current() {
            Some(T::Lifetime) => {
                lifetime(p);
                if p.eat(T::Colon) {
                    bounds(p);
                }
                K::LifetimeParam
            }
            Some(T::Const) => {
                p.bump();
                name(p);
                if p.eat(T::Colon) {
                    type_(p);
                }
                if p.eat(T::Eq) {
                    if p.at(T::LBrace) {
                        super::stmts::block(p);
                    } else if p.eat(T::Minus) {
                        if p.current().is_some_and(|c| c.is_literal() || matches!(c, T::True | T::False)) {
                            p.bump();
                        }
                    } else if p.current().is_some_and(|c| c.is_literal() || matches!(c, T::True | T::False | T::Ident)) {
                        p.bump();
                    }
                }
                K::ConstParam
            }
            _ => {
                name(p);
                if p.eat(T::Colon) {
                    bounds(p);
                }
                if p.eat(T::Eq) {
                    type_(p);
                }
                K::TypeParam
            }
        };
        g.complete(p, kind);
        if !p.eat(T::Comma) {
            if p.mark_pos() == before {
                p.err_bump();
            } else {
                break;
            }
        }
    }
    p.expect(T::Gt);
    m.complete(p, K::GenericParamList);
}

/// `where T: Bound, 'a: 'b, for<'x> F: Fn(&'x u8)`.
pub(crate) fn opt_where_clause(p: &mut Parser<'_>) {
    if !p.at(T::Where) {
        return;
    }
    let m = p.start();
    p.bump();
    while !p.at_eof() && !matches!(p.current(), Some(T::LBrace | T::Semi | T::Eq)) {
        let before = p.mark_pos();
        let w = p.start();
        if p.at(T::Lifetime) {
            lifetime(p);
        } else {
            opt_for_binder(p);
            type_(p);
        }
        if p.eat(T::Colon) {
            bounds(p);
        }
        w.complete(p, K::WherePred);
        if !p.eat(T::Comma) {
            if p.mark_pos() == before {
                p.err_bump();
            } else {
                break;
            }
        }
    }
    m.complete(p, K::WhereClause);
}

pub(crate) fn opt_for_binder(p: &mut Parser<'_>) {
    if p.at(T::For) && p.nth_at(1, T::Lt) {
        let m = p.start();
        p.bump();
        opt_generic_params(p);
        m.complete(p, K::ForBinder);
    }
}

/// `Bound + 'a + ?Sized + for<'x> Fn(&'x T)`.
pub(crate) fn bounds(p: &mut Parser<'_>) {
    let m = p.start();
    loop {
        let b = p.start();
        // `[const] Trait` of const trait bounds.
        if p.at(T::LBracket) && p.nth_at(1, T::Const) && p.nth_at(2, T::RBracket) {
            p.bump_n(3);
        }
        match p.current() {
            Some(T::Lifetime) => lifetime(p),
            Some(T::LParen) => token_tree(p),
            Some(
                T::Question
                | T::Tilde
                | T::Const
                | T::Async
                | T::For
                | T::Ident
                | T::RawIdent
                | T::SelfType
                | T::SelfKw
                | T::Super
                | T::Crate
                | T::Lt
                | T::Colon
                | T::Use
                | T::Bang,
            ) => {
                p.eat(T::Question);
                if p.eat(T::Tilde) {
                    p.eat(T::Const);
                }
                p.eat(T::Const);
                p.eat(T::Async);
                p.eat(T::Bang);
                opt_for_binder(p);
                if p.at(T::Use) {
                    p.bump();
                    if p.at(T::Lt) {
                        generic_args(p);
                    }
                } else {
                    path(p, PathMode::Type);
                }
            }
            _ => {
                b.abandon(p);
                break;
            }
        }
        b.complete(p, K::TypeBound);
        if !p.eat(T::Plus) {
            break;
        }
    }
    m.complete(p, K::TypeBoundList);
}
