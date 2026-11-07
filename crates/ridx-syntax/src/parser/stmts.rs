/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Blocks and statements: `let` (with `else`), items, expression statements and the tail expression.

use super::expr::{expr, expr_bp, Restrictions};
use super::{items, patterns, types, Parser};
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;

/// `{ stmts }`, completed as a `BlockExpr`. Without a `{` it leaves an `Error`.
pub(crate) fn block(p: &mut Parser<'_>) {
    if !p.at(T::LBrace) {
        p.expect(T::LBrace);
        return;
    }
    let m = p.start();
    block_body(p);
    m.complete(p, K::BlockExpr);
}

/// The `{ .. }` of a block, its node opened by the caller.
pub(crate) fn block_body(p: &mut Parser<'_>) {
    p.bump();
    items::inner_attrs(p);
    while !p.at_eof() && !p.at(T::RBrace) {
        let before = p.mark_pos();
        stmt(p);
        if p.mark_pos() == before {
            p.err_bump();
        }
    }
    p.expect(T::RBrace);
}

fn stmt(p: &mut Parser<'_>) {
    if p.eat(T::Semi) {
        return;
    }
    let m = p.start();
    let attr_pos = p.mark_pos();
    items::attrs(p);
    let has_attrs = p.mark_pos() != attr_pos;
    if p.at(T::Let) {
        p.bump();
        patterns::pattern(p);
        if p.eat(T::Colon) {
            types::type_(p);
        }
        if p.eat(T::Eq) {
            expr(p);
            if p.at(T::Else) {
                let e = p.start();
                p.bump();
                block(p);
                e.complete(p, K::LetElse);
            }
        }
        p.expect(T::Semi);
        m.complete(p, K::LetStmt);
        return;
    }
    if items::at_item_start(p) {
        m.abandon(p);
        // The attributes were consumed already; the item node starts after them.
        items::item(p);
        return;
    }
    let Some(_) = expr_bp(p, 1, Restrictions { no_struct: false, stmt: true }) else {
        if has_attrs {
            m.complete(p, K::Error);
        } else {
            m.abandon(p);
        }
        return;
    };
    if p.eat(T::Semi) {
        m.complete(p, K::ExprStmt);
    } else if p.at(T::RBrace) {
        // The tail expression: left as the block's direct child.
        m.abandon(p);
    } else {
        // Block-like statements and brace macros need no `;`; anything else is missing one, kept as written.
        m.complete(p, K::ExprStmt);
    }
}
