/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Expressions by precedence climbing: binary and range operators, casts, prefix operators and postfix calls,
//! method calls, fields, indexing, `?` and `.await`.

use super::{expr_atoms, types, CompletedMarker, Parser};
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;

/// What an expression may contain where it is parsed.
#[derive(Clone, Copy, Default)]
pub(crate) struct Restrictions {
    /// No struct literal: `if x == S {}` is a comparison with a block, not `S {}`.
    pub no_struct: bool,
    /// At the start of a statement: a block-like expression ends it (`match x {} - 1` is two statements).
    pub stmt: bool,
}

pub(crate) fn expr(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    expr_bp(p, 1, Restrictions::default())
}

/// An expression without struct literals (conditions, scrutinees, `for` iterables).
pub(crate) fn expr_no_struct(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    expr_bp(p, 1, Restrictions { no_struct: true, stmt: false })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Op {
    Assign,
    Range,
    Bin,
    Cast,
}

/// The operator ahead: its binding power, token count and kind. `=>` and `||`-closures are not operators.
fn current_op(p: &mut Parser<'_>) -> Option<(u8, usize, Op)> {
    const OPS3: &[(&str, u8, Op)] = &[("<<=", 1, Op::Assign), (">>=", 1, Op::Assign), ("..=", 2, Op::Range), ("...", 2, Op::Range)];
    const OPS2: &[(&str, u8, Op)] = &[
        ("||", 3, Op::Bin),
        ("&&", 4, Op::Bin),
        ("==", 5, Op::Bin),
        ("!=", 5, Op::Bin),
        ("<=", 5, Op::Bin),
        (">=", 5, Op::Bin),
        ("<<", 9, Op::Bin),
        (">>", 9, Op::Bin),
        ("+=", 1, Op::Assign),
        ("-=", 1, Op::Assign),
        ("*=", 1, Op::Assign),
        ("/=", 1, Op::Assign),
        ("%=", 1, Op::Assign),
        ("^=", 1, Op::Assign),
        ("&=", 1, Op::Assign),
        ("|=", 1, Op::Assign),
        ("..", 2, Op::Range),
    ];
    if p.at(T::As) {
        return Some((12, 1, Op::Cast));
    }
    if !p.current().is_some_and(|k| k.is_punct()) {
        return None;
    }
    if p.at_op("=>") || p.at_op("->") {
        return None;
    }
    for &(s, bp, op) in OPS3 {
        if p.at_op(s) {
            return Some((bp, 3, op));
        }
    }
    for &(s, bp, op) in OPS2 {
        if p.at_op(s) {
            return Some((bp, 2, op));
        }
    }
    let (bp, op) = match p.current()? {
        T::Eq => (1, Op::Assign),
        T::Lt | T::Gt => (5, Op::Bin),
        T::Pipe => (6, Op::Bin),
        T::Caret => (7, Op::Bin),
        T::Amp => (8, Op::Bin),
        T::Plus | T::Minus => (10, Op::Bin),
        T::Star | T::Slash | T::Percent => (11, Op::Bin),
        _ => return None,
    };
    Some((bp, 1, op))
}

/// Whether an expression can start here (the optional operand of `return`, `break`, a range end).
pub(crate) fn at_expr_start(p: &mut Parser<'_>) -> bool {
    match p.current() {
        None => false,
        Some(k) if k.is_literal() => true,
        Some(
            T::Ident
            | T::RawIdent
            | T::SelfKw
            | T::SelfType
            | T::Super
            | T::Crate
            | T::LParen
            | T::LBracket
            | T::LBrace
            | T::Bang
            | T::Minus
            | T::Star
            | T::Amp
            | T::Pipe
            | T::Lt
            | T::Underscore
            | T::True
            | T::False
            | T::If
            | T::Match
            | T::Loop
            | T::While
            | T::For
            | T::Return
            | T::Break
            | T::Continue
            | T::Unsafe
            | T::Async
            | T::Move
            | T::Static
            | T::Const
            | T::Let
            | T::Lifetime,
        ) => true,
        Some(T::Colon) => p.at_op("::"),
        Some(T::Dot) => p.at_op(".."),
        _ => false,
    }
}

/// An expression whose operators bind at least as tight as `min_bp`.
pub(crate) fn expr_bp(p: &mut Parser<'_>, min_bp: u8, r: Restrictions) -> Option<CompletedMarker> {
    let mut lhs = if p.at_op("..") && !p.at_op("...") {
        // `..`, `..end`, `..=end`.
        let m = p.start();
        let n = if p.at_op("..=") { 3 } else { 2 };
        p.bump_n(n);
        if at_expr_start(p) && !(r.no_struct && p.at(T::LBrace)) {
            expr_bp(p, 3, Restrictions { stmt: false, ..r });
        }
        m.complete(p, K::RangeExpr)
    } else {
        let lhs = unary(p, r)?;
        if r.stmt && lhs.kind().is_block_like() {
            return Some(lhs);
        }
        lhs
    };
    let r = Restrictions { stmt: false, ..r };
    while let Some((bp, n, op)) = current_op(p) {
        if bp < min_bp {
            break;
        }
        let m = lhs.precede(p);
        p.bump_n(n);
        lhs = match op {
            Op::Cast => {
                types::type_no_bounds(p);
                m.complete(p, K::CastExpr)
            }
            Op::Range => {
                if at_expr_start(p) && !(r.no_struct && p.at(T::LBrace)) {
                    expr_bp(p, bp + 1, r);
                }
                m.complete(p, K::RangeExpr)
            }
            Op::Assign => {
                expr_bp(p, bp, r);
                m.complete(p, K::BinExpr)
            }
            Op::Bin => {
                expr_bp(p, bp + 1, r);
                m.complete(p, K::BinExpr)
            }
        };
    }
    Some(lhs)
}

/// Prefix operators over a postfix expression: `-x`, `!x`, `*x`, `&mut x`, `&raw const x`.
fn unary(p: &mut Parser<'_>, r: Restrictions) -> Option<CompletedMarker> {
    let stmt = r.stmt;
    let r = Restrictions { stmt: false, ..r };
    match p.current() {
        Some(T::Minus | T::Bang | T::Star) => {
            let m = p.start();
            p.bump();
            unary(p, r);
            Some(m.complete(p, K::PrefixExpr))
        }
        Some(T::Amp) => {
            let m = p.start();
            p.bump();
            if p.at_contextual("raw") && matches!(p.nth(1), Some(T::Const | T::Mut)) {
                p.bump_n(2);
            } else {
                p.eat(T::Mut);
            }
            unary(p, r);
            Some(m.complete(p, K::RefExpr))
        }
        _ => {
            let atom = expr_atoms::atom(p, r)?;
            // A block-like statement (`if c {}`) is not called by a `(..)` on the next line, but may take a
            // method call: `match x {}.len()`.
            if stmt && atom.kind().is_block_like() && !(p.at(T::Dot) && !p.at_op("..")) && !p.at(T::Question) {
                return Some(atom);
            }
            Some(postfix(p, atom))
        }
    }
}

/// Calls, method calls, fields, indexing, `?` and `.await` after `lhs`.
fn postfix(p: &mut Parser<'_>, mut lhs: CompletedMarker) -> CompletedMarker {
    loop {
        lhs = match p.current() {
            Some(T::LParen) => {
                let m = lhs.precede(p);
                arg_list(p);
                m.complete(p, K::CallExpr)
            }
            Some(T::LBracket) => {
                let m = lhs.precede(p);
                p.bump();
                expr(p);
                p.expect(T::RBracket);
                m.complete(p, K::IndexExpr)
            }
            Some(T::Question) => {
                let m = lhs.precede(p);
                p.bump();
                m.complete(p, K::TryExpr)
            }
            Some(T::Dot) if !p.at_op("..") => {
                let m = lhs.precede(p);
                p.bump();
                match p.current() {
                    Some(T::Await) => {
                        p.bump();
                        m.complete(p, K::AwaitExpr)
                    }
                    Some(T::Ident | T::RawIdent) => {
                        super::name(p);
                        if p.at_op("::") && p.nth_at(2, T::Lt) {
                            p.bump_n(2);
                            types::generic_args(p);
                        }
                        if p.at(T::LParen) {
                            arg_list(p);
                            m.complete(p, K::MethodCallExpr)
                        } else {
                            m.complete(p, K::FieldExpr)
                        }
                    }
                    // `t.0`, and `t.0.1` lexed as the float `0.1`.
                    Some(T::Int | T::Float) => {
                        p.bump();
                        m.complete(p, K::FieldExpr)
                    }
                    _ => {
                        p.expect(T::Ident);
                        m.complete(p, K::FieldExpr)
                    }
                }
            }
            _ => return lhs,
        };
    }
}

/// `(a, b)` of a call.
pub(crate) fn arg_list(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump();
    while !p.at_eof() && !p.at(T::RParen) {
        let before = p.mark_pos();
        super::items::attrs(p);
        expr(p);
        if !p.eat(T::Comma) {
            if p.mark_pos() == before {
                p.err_bump();
            } else {
                break;
            }
        }
    }
    p.expect(T::RParen);
    m.complete(p, K::ArgList);
}
