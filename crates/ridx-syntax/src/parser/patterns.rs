/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Patterns: bindings, wildcards, references, tuples, structs, slices, literals, ranges and alternatives.

use super::{name, token_tree, types, Parser};
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;

/// A pattern with alternatives: `A | B`.
pub(crate) fn pattern(p: &mut Parser<'_>) {
    let m = p.start();
    let leading = p.at(T::Pipe) && !p.at_op("||");
    if leading {
        p.bump();
    }
    pattern_single(p);
    let mut alts = leading;
    while p.at(T::Pipe) && !p.at_op("||") {
        p.bump();
        pattern_single(p);
        alts = true;
    }
    if alts {
        m.complete(p, K::OrPat);
    } else {
        m.abandon(p);
    }
}

/// A pattern where a `|` belongs to the context (closure parameters, function parameters).
pub(crate) fn pattern_no_top_alt(p: &mut Parser<'_>) {
    pattern_single(p);
}

fn at_range_op(p: &mut Parser<'_>) -> Option<usize> {
    if p.at_op("..=") || p.at_op("...") {
        Some(3)
    } else if p.at_op("..") {
        Some(2)
    } else {
        None
    }
}

/// Whether a range pattern's end follows (a literal, `-literal` or a path), not `,` `)` `]` `|` `=>`.
fn at_range_end(p: &mut Parser<'_>) -> bool {
    match p.current() {
        Some(k) if k.is_literal() => true,
        Some(T::Minus) => true,
        Some(T::Ident | T::RawIdent | T::SelfType | T::Super | T::Crate | T::Lt) => true,
        Some(T::Colon) => p.at_op("::"),
        _ => false,
    }
}

fn pattern_single(p: &mut Parser<'_>) {
    let m = p.start();
    let kind = match p.current() {
        Some(T::Underscore) => {
            p.bump();
            K::WildcardPat
        }
        Some(T::Dot) if at_range_op(p).is_some() => {
            let n = at_range_op(p).unwrap_or(2);
            p.bump_n(n);
            if at_range_end(p) {
                range_end(p);
                K::RangePat
            } else {
                K::RestPat
            }
        }
        Some(T::Amp) => {
            p.bump();
            p.eat(T::Mut);
            pattern_single(p);
            K::RefPat
        }
        Some(T::LParen) => {
            let (n, trailing) = seq(p, T::RParen);
            if n == 1 && !trailing {
                K::ParenPat
            } else {
                K::TuplePat
            }
        }
        Some(T::LBracket) => {
            seq(p, T::RBracket);
            K::SlicePat
        }
        Some(T::Const) if p.nth_at(1, T::LBrace) => {
            p.bump();
            super::stmts::block(p);
            K::ConstBlockPat
        }
        Some(T::Ref | T::Mut) => {
            p.eat(T::Ref);
            p.eat(T::Mut);
            name(p);
            opt_at_subpattern(p);
            K::IdentPat
        }
        Some(T::Ident)
            if p.nth_text(0) == "box" && !matches!(p.nth(1), Some(T::Comma | T::RParen | T::RBracket | T::Colon | T::Eq | T::Pipe | T::At | T::Semi)) && !p.nth_at(1, T::Colon) =>
        {
            p.bump();
            pattern_single(p);
            K::BoxPat
        }
        Some(k) if k.is_literal() || matches!(k, T::Minus | T::True | T::False) => {
            if p.eat(T::Minus) {
                if p.current().is_some_and(|c| c.is_literal() || matches!(c, T::True | T::False)) {
                    p.bump();
                }
            } else {
                p.bump();
            }
            maybe_range(p, K::LiteralPat)
        }
        Some(T::Ident | T::RawIdent) if !matches!(p.nth(1), Some(T::LParen | T::LBrace | T::Bang)) && !p.nth_at_op(1, "::") => {
            name(p);
            if at_range_op(p).is_some() {
                // `a..=b` with `a` a constant.
                maybe_range(p, K::IdentPat)
            } else {
                opt_at_subpattern(p);
                K::IdentPat
            }
        }
        Some(T::Ident | T::RawIdent | T::SelfType | T::SelfKw | T::Super | T::Crate | T::Lt | T::Colon) => {
            types::path(p, types::PathMode::Expr);
            match p.current() {
                Some(T::LParen) => {
                    seq(p, T::RParen);
                    K::TupleStructPat
                }
                Some(T::LBrace) => {
                    record_fields(p);
                    K::RecordPat
                }
                Some(T::Bang) => {
                    p.bump();
                    token_tree(p);
                    K::MacroPat
                }
                _ => maybe_range(p, K::PathPat),
            }
        }
        _ => K::Error,
    };
    m.complete(p, kind);
}

/// After a range start: `..=end` turns the pattern into a range, else it stays `kind`.
fn maybe_range(p: &mut Parser<'_>, kind: K) -> K {
    let Some(n) = at_range_op(p) else { return kind };
    p.bump_n(n);
    if at_range_end(p) {
        range_end(p);
    }
    K::RangePat
}

fn range_end(p: &mut Parser<'_>) {
    let m = p.start();
    if p.eat(T::Minus) {
        if p.current().is_some_and(|k| k.is_literal()) {
            p.bump();
        }
        m.complete(p, K::LiteralPat);
    } else if p.current().is_some_and(|k| k.is_literal()) {
        p.bump();
        m.complete(p, K::LiteralPat);
    } else {
        types::path(p, types::PathMode::Expr);
        m.complete(p, K::PathPat);
    }
}

fn opt_at_subpattern(p: &mut Parser<'_>) {
    if p.eat(T::At) {
        pattern_single(p);
    }
}

/// `(a, b)` or `[a, b]`: the number of patterns and whether a comma trails.
fn seq(p: &mut Parser<'_>, close: T) -> (usize, bool) {
    p.bump();
    let mut n = 0;
    let mut trailing = false;
    while !p.at_eof() && !p.at(close) {
        let before = p.mark_pos();
        pattern(p);
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
    p.expect(close);
    (n, trailing)
}

/// `{ a, b: pat, ref mut c, 0: x, .. }`.
fn record_fields(p: &mut Parser<'_>) {
    p.bump();
    while !p.at_eof() && !p.at(T::RBrace) {
        let before = p.mark_pos();
        let f = p.start();
        super::items::attrs(p);
        if p.at_op("..") {
            p.bump_n(2);
            f.complete(p, K::RestPat);
        } else {
            if matches!(p.current(), Some(T::Ident | T::RawIdent | T::Int)) && p.nth_at(1, T::Colon) && !p.nth_at_op(1, "::") {
                p.bump();
                p.bump();
                pattern(p);
            } else {
                let b = p.start();
                if p.at_contextual("box") && matches!(p.nth(1), Some(T::Ident | T::Ref | T::Mut)) {
                    p.bump();
                }
                p.eat(T::Ref);
                p.eat(T::Mut);
                name(p);
                b.complete(p, K::IdentPat);
            }
            f.complete(p, K::RecordPatField);
        }
        if !p.eat(T::Comma) {
            if p.mark_pos() == before {
                p.err_bump();
            } else {
                break;
            }
        }
    }
    p.expect(T::RBrace);
}
