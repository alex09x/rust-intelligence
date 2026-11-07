/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Types and paths.

use super::{at_name_ref, token_tree, Parser};
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;

/// Where a path appears: in a type `<..>` opens generic arguments, in an expression only `::<..>` does.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PathMode {
    Type,
    Expr,
}

/// A path: `a::b`, `::std::x`, `<T as Trait>::Item`, `Vec<u8>`, `Fn(A) -> B`.
pub(crate) fn path(p: &mut Parser<'_>, mode: PathMode) {
    let m = p.start();
    if p.at(T::Lt) {
        // Qualified path: `<Type as Trait>::`.
        let s = p.start();
        p.bump();
        type_(p);
        if p.eat(T::As) {
            path(p, PathMode::Type);
        }
        p.expect(T::Gt);
        s.complete(p, K::PathSegment);
    } else {
        if p.at_op("::") {
            p.bump_n(2);
        }
        segment(p, mode);
    }
    // A `::` not followed by a name (`use a::{b}`, `a::*`) ends the path.
    while p.at_op("::") && matches!(p.nth(2), Some(T::Ident | T::RawIdent | T::SelfKw | T::SelfType | T::Super | T::Crate)) {
        p.bump_n(2);
        segment(p, mode);
    }
    m.complete(p, K::Path);
}

fn segment(p: &mut Parser<'_>, mode: PathMode) {
    let m = p.start();
    if at_name_ref(p) {
        p.bump();
    } else {
        p.expect(T::Ident);
    }
    match mode {
        PathMode::Type => {
            if p.at_op("::") && p.nth_at(2, T::Lt) {
                p.bump_n(2);
            }
            if p.at(T::Lt) && !p.at_op("<=") {
                generic_args(p);
            } else if p.at(T::LParen) {
                // `Fn(A, B) -> C`.
                tuple_or_paren_type(p);
                super::items::opt_ret_type(p);
            }
        }
        PathMode::Expr => {
            if p.at_op("::") && p.nth_at(2, T::Lt) {
                p.bump_n(2);
                generic_args(p);
            }
        }
    }
    m.complete(p, K::PathSegment);
}
pub(crate) use super::generics::{bounds, generic_args, lifetime, opt_for_binder, opt_generic_params, opt_where_clause};

/// A type, which may be a bare `Trait + Send` object.
pub(crate) fn type_(p: &mut Parser<'_>) {
    type_inner(p, true);
}

/// A type where a `+` belongs to the context (`&dyn A + B` is not allowed, `impl Fn() -> T + Send` is the impl's).
pub(crate) fn type_no_bounds(p: &mut Parser<'_>) {
    type_inner(p, false);
}

fn type_inner(p: &mut Parser<'_>, allow_bounds: bool) {
    let m = p.start();
    let kind = match p.current() {
        Some(T::LParen) => {
            m.abandon(p);
            tuple_or_paren_type(p);
            return;
        }
        Some(T::Bang) => {
            p.bump();
            K::NeverType
        }
        Some(T::Amp) => {
            p.bump();
            if p.at(T::Lifetime) {
                lifetime(p);
            }
            p.eat(T::Mut);
            type_no_bounds(p);
            K::RefType
        }
        Some(T::Star) => {
            p.bump();
            if !p.eat(T::Mut) {
                p.eat(T::Const);
            }
            type_no_bounds(p);
            K::PtrType
        }
        Some(T::LBracket) => {
            p.bump();
            type_(p);
            let kind = if p.eat(T::Semi) {
                super::expr::expr(p);
                K::ArrayType
            } else {
                K::SliceType
            };
            p.expect(T::RBracket);
            kind
        }
        Some(T::Underscore) => {
            p.bump();
            K::InferType
        }
        Some(T::Fn | T::Unsafe | T::Extern) => {
            fn_ptr(p);
            K::FnPtrType
        }
        Some(T::For) if p.nth_at(1, T::Lt) => {
            opt_for_binder(p);
            if matches!(p.current(), Some(T::Fn | T::Unsafe | T::Extern)) {
                fn_ptr(p);
                K::FnPtrType
            } else {
                bounds(p);
                K::DynTraitType
            }
        }
        Some(T::Impl) => {
            p.bump();
            if allow_bounds {
                bounds(p)
            } else {
                single_bound(p)
            }
            K::ImplTraitType
        }
        Some(T::Dyn) => {
            p.bump();
            if allow_bounds {
                bounds(p)
            } else {
                single_bound(p)
            }
            K::DynTraitType
        }
        Some(T::Question) => {
            bounds(p);
            K::DynTraitType
        }
        Some(T::Lt | T::Ident | T::RawIdent | T::SelfType | T::SelfKw | T::Super | T::Crate | T::Colon) => {
            path(p, PathMode::Type);
            if p.at(T::Bang) {
                p.bump();
                token_tree(p);
                K::MacroCall
            } else if allow_bounds && p.at(T::Plus) {
                // A bare trait object: `Trait + Send`.
                let c = m.complete(p, K::PathType);
                let o = c.precede(p);
                while p.eat(T::Plus) {
                    bounds(p);
                }
                o.complete(p, K::DynTraitType);
                return;
            } else {
                K::PathType
            }
        }
        _ => {
            m.complete(p, K::Error);
            return;
        }
    };
    m.complete(p, kind);
}

fn single_bound(p: &mut Parser<'_>) {
    let m = p.start();
    let b = p.start();
    if p.at(T::Lifetime) {
        lifetime(p);
    } else {
        p.eat(T::Question);
        opt_for_binder(p);
        path(p, PathMode::Type);
    }
    b.complete(p, K::TypeBound);
    m.complete(p, K::TypeBoundList);
}

fn fn_ptr(p: &mut Parser<'_>) {
    p.eat(T::Unsafe);
    if p.at(T::Extern) {
        let a = p.start();
        p.bump();
        if p.current().is_some_and(|k| k.is_literal()) {
            p.bump();
        }
        a.complete(p, K::Abi);
    }
    p.expect(T::Fn);
    if p.at(T::LParen) {
        token_tree(p);
    }
    super::items::opt_ret_type(p);
}

/// `()`, `(T)`, `(A, B)`.
fn tuple_or_paren_type(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump();
    let mut n = 0;
    let mut trailing_comma = false;
    while !p.at_eof() && !p.at(T::RParen) {
        let before = p.mark_pos();
        type_(p);
        n += 1;
        trailing_comma = p.eat(T::Comma);
        if !trailing_comma {
            if p.mark_pos() == before {
                p.err_bump();
            } else {
                break;
            }
        }
    }
    p.expect(T::RParen);
    m.complete(p, if n == 1 && !trailing_comma { K::ParenType } else { K::TupleType });
}
