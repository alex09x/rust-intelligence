/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Expression atoms: literals, paths, struct literals, macro calls, groups, blocks, control flow and closures.

use super::expr::{at_expr_start, expr, expr_bp, expr_no_struct, Restrictions};
use super::{patterns, stmts, token_tree, types, CompletedMarker, Parser};
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;

/// An atom, or `None` with nothing consumed when no expression starts here.
pub(crate) fn atom(p: &mut Parser<'_>, r: Restrictions) -> Option<CompletedMarker> {
    let m = p.start();
    let kind = match p.current() {
        Some(k) if k.is_literal() || matches!(k, T::True | T::False) => {
            p.bump();
            K::Literal
        }
        Some(T::Ident) => {
            let t = p.tok(0).unwrap();
            match t.end - t.start {
                3 if p.nth_at(1, T::LBrace) && &p.src.as_bytes()[t.start as usize..t.end as usize] == b"try" => {
                    p.bump();
                    stmts::block_body(p);
                    K::BlockExpr
                }
                5 if !matches!(p.nth(1), Some(T::Colon | T::Bang | T::LParen | T::Dot)) && &p.src.as_bytes()[t.start as usize..t.end as usize] == b"yield" => {
                    p.bump();
                    if at_expr_start(p) {
                        expr(p);
                    }
                    K::YieldExpr
                }
                6 if &p.src.as_bytes()[t.start as usize..t.end as usize] == b"become" && at_ident_or_path(p, 1) => {
                    p.bump();
                    expr(p);
                    K::BecomeExpr
                }
                _ => path_expr(p, r),
            }
        }
        Some(T::RawIdent | T::SelfKw | T::SelfType | T::Super | T::Crate | T::Lt) => path_expr(p, r),
        Some(T::Colon) if p.at_op("::") => path_expr(p, r),
        Some(T::LParen) => paren_or_tuple(p),
        Some(T::LBracket) => {
            p.bump();
            while !p.at_eof() && !p.at(T::RBracket) {
                let before = p.mark_pos();
                super::items::attrs(p);
                expr(p);
                if p.eat(T::Semi) {
                    expr(p);
                    break;
                }
                if !p.eat(T::Comma) {
                    if p.mark_pos() == before {
                        p.err_bump();
                    } else {
                        break;
                    }
                }
            }
            p.expect(T::RBracket);
            K::ArrayExpr
        }
        // Blocks: the node opened here is the block, its body parsed into it.
        Some(T::LBrace) => {
            stmts::block_body(p);
            K::BlockExpr
        }
        Some(T::Unsafe | T::Const) if p.nth_at(1, T::LBrace) => {
            p.bump();
            stmts::block_body(p);
            K::BlockExpr
        }
        Some(T::Async) if p.nth_at(1, T::LBrace) || (p.nth_at(1, T::Move) && p.nth_at(2, T::LBrace)) => {
            p.bump();
            p.eat(T::Move);
            stmts::block_body(p);
            K::BlockExpr
        }
        Some(T::Lifetime) if p.nth_at(1, T::Colon) => {
            let l = p.start();
            p.bump_n(2);
            l.complete(p, K::Label);
            return labeled(p, m);
        }
        Some(T::If) => {
            if_(p);
            K::IfExpr
        }
        Some(T::Match) => {
            match_(p);
            K::MatchExpr
        }
        Some(T::Loop | T::While | T::For) => return labeled(p, m),
        Some(T::Let) => {
            p.bump();
            patterns::pattern(p);
            p.expect(T::Eq);
            // `let` chains: `let a = b && c` is `(let a = b) && c`.
            expr_bp(p, 5, Restrictions { no_struct: true, stmt: false });
            K::LetExpr
        }
        Some(T::Return) => {
            p.bump();
            if at_expr_start(p) && !(r.no_struct && p.at(T::LBrace)) {
                expr_bp(p, 1, Restrictions { stmt: false, ..r });
            }
            K::ReturnExpr
        }
        Some(T::Break) => {
            p.bump();
            if p.at(T::Lifetime) {
                types::lifetime(p);
            }
            if at_expr_start(p) && !(r.no_struct && p.at(T::LBrace)) {
                expr_bp(p, 1, Restrictions { stmt: false, ..r });
            }
            K::BreakExpr
        }
        Some(T::Continue) => {
            p.bump();
            if p.at(T::Lifetime) {
                types::lifetime(p);
            }
            K::ContinueExpr
        }
        Some(T::Underscore) => {
            p.bump();
            K::UnderscoreExpr
        }
        Some(T::Pipe | T::Move | T::Static | T::Async) => {
            closure(p);
            K::ClosureExpr
        }
        _ => {
            m.abandon(p);
            return None;
        }
    };
    Some(m.complete(p, kind))
}

fn at_ident_or_path(p: &mut Parser<'_>, n: usize) -> bool {
    matches!(p.nth(n), Some(T::Ident | T::RawIdent | T::SelfKw | T::SelfType | T::Super | T::Crate))
}

/// A path, then a macro call `path!(..)` or a struct literal `Path { .. }`.
fn path_expr(p: &mut Parser<'_>, r: Restrictions) -> K {
    types::path(p, types::PathMode::Expr);
    if p.at(T::Bang) && !p.at_op("!=") && matches!(p.nth(1), Some(T::LParen | T::LBrace | T::LBracket)) {
        p.bump();
        token_tree(p);
        return K::MacroCall;
    }
    if p.at(T::LBrace) && !r.no_struct {
        record_fields(p);
        return K::RecordExpr;
    }
    K::PathExpr
}

/// `{ a, b: expr, ..base }` of a struct literal.
fn record_fields(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump();
    while !p.at_eof() && !p.at(T::RBrace) {
        let before = p.mark_pos();
        if p.at_op("..") {
            p.bump_n(2);
            if at_expr_start(p) {
                expr(p);
            }
            break;
        }
        let f = p.start();
        super::items::attrs(p);
        if matches!(p.current(), Some(T::Ident | T::RawIdent | T::Int)) && p.nth_at(1, T::Colon) && !p.nth_at_op(1, "::") {
            p.bump_n(2);
            expr(p);
        } else {
            expr(p);
        }
        f.complete(p, K::RecordExprField);
        if !p.eat(T::Comma) {
            if p.mark_pos() == before {
                p.err_bump();
            } else {
                break;
            }
        }
    }
    p.expect(T::RBrace);
    m.complete(p, K::RecordExprFieldList);
}

fn paren_or_tuple(p: &mut Parser<'_>) -> K {
    p.bump();
    let mut n = 0;
    let mut trailing = false;
    while !p.at_eof() && !p.at(T::RParen) {
        let before = p.mark_pos();
        super::items::attrs(p);
        expr(p);
        n += 1;
        trailing = p.eat(T::Comma);
        if !trailing {
            if p.mark_pos() == before {
                p.err_bump();
            } else {
                break;
            }
        }
    }
    p.expect(T::RParen);
    if n == 1 && !trailing {
        K::ParenExpr
    } else {
        K::TupleExpr
    }
}

/// `loop`, `while`, `for` or a labeled block after an optional label.
fn labeled(p: &mut Parser<'_>, m: super::Marker) -> Option<CompletedMarker> {
    let kind = match p.current() {
        Some(T::Loop) => {
            p.bump();
            stmts::block(p);
            K::LoopExpr
        }
        Some(T::While) => {
            p.bump();
            expr_no_struct(p);
            stmts::block(p);
            K::WhileExpr
        }
        Some(T::For) if !p.nth_at(1, T::Lt) => {
            p.bump();
            patterns::pattern(p);
            p.expect(T::In);
            expr_no_struct(p);
            stmts::block(p);
            K::ForExpr
        }
        Some(T::For) => {
            // `for<'a> |x: &'a u8| ..`.
            closure(p);
            K::ClosureExpr
        }
        Some(T::LBrace) => {
            stmts::block_body(p);
            K::BlockExpr
        }
        Some(T::Unsafe) if p.nth_at(1, T::LBrace) => {
            p.bump();
            stmts::block_body(p);
            K::BlockExpr
        }
        _ => K::Error,
    };
    Some(m.complete(p, kind))
}

fn if_(p: &mut Parser<'_>) {
    p.bump();
    expr_no_struct(p);
    stmts::block(p);
    if p.eat(T::Else) {
        if p.at(T::If) {
            let m = p.start();
            if_(p);
            m.complete(p, K::IfExpr);
        } else {
            stmts::block(p);
        }
    }
}

fn match_(p: &mut Parser<'_>) {
    p.bump();
    expr_no_struct(p);
    if !p.at(T::LBrace) {
        p.expect(T::LBrace);
        return;
    }
    let l = p.start();
    p.bump();
    super::items::attrs(p);
    while !p.at_eof() && !p.at(T::RBrace) {
        let before = p.mark_pos();
        let arm = p.start();
        super::items::attrs(p);
        patterns::pattern(p);
        if p.at(T::If) {
            let g = p.start();
            p.bump();
            expr(p);
            g.complete(p, K::MatchGuard);
        }
        if !p.eat_op("=>") {
            p.expect(T::Eq);
        }
        let body = expr_bp(p, 1, Restrictions { no_struct: false, stmt: true });
        let block_like = body.is_some_and(|b| b.kind().is_block_like());
        arm.complete(p, K::MatchArm);
        if !p.eat(T::Comma) && !block_like && !p.at(T::RBrace) && p.mark_pos() == before {
            p.err_bump();
        }
    }
    p.expect(T::RBrace);
    l.complete(p, K::MatchArmList);
}

/// `|a, b: T| body`, `move ||`, `async move |x|`, `for<'a> |x: &'a u8|`, `static ||`.
fn closure(p: &mut Parser<'_>) {
    if p.eat(T::For) {
        types::opt_generic_params(p);
    }
    p.eat(T::Static);
    p.eat(T::Async);
    p.eat(T::Move);
    let pl = p.start();
    if p.at_op("||") {
        p.bump_n(2);
    } else if p.eat(T::Pipe) {
        while !p.at_eof() && !p.at(T::Pipe) {
            let before = p.mark_pos();
            let param = p.start();
            super::items::attrs(p);
            patterns::pattern_no_top_alt(p);
            if p.eat(T::Colon) {
                types::type_no_bounds(p);
            }
            param.complete(p, K::Param);
            if !p.eat(T::Comma) {
                if p.mark_pos() == before {
                    p.err_bump();
                } else {
                    break;
                }
            }
        }
        p.expect(T::Pipe);
    } else {
        p.expect(T::Pipe);
    }
    pl.complete(p, K::ParamList);
    if p.at_op("->") {
        super::items::opt_ret_type(p);
        stmts::block(p);
    } else {
        expr(p);
    }
}
