/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Items: functions with their signatures, impls and traits with their member lists, and the other items, whose
//! bodies (struct fields, enum variants, `use` trees) are kept as token trees.

use super::items_fn::fn_;
pub(crate) use super::items_fn::opt_ret_type;
use super::{name, token_tree, types, CompletedMarker, Parser};
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;

/// Items until the end of input, or until the closing `}` when `in_braces`.
pub(crate) fn item_list(p: &mut Parser<'_>, in_braces: bool) {
    inner_attrs(p);
    loop {
        match p.current() {
            None => return,
            Some(T::RBrace) if in_braces => return,
            _ => {}
        }
        if super::items_fn::try_skip_non_target_item(p) {
            continue;
        }
        let before = p.mark_pos();
        if item(p).is_none() && p.mark_pos() == before {
            p.err_bump();
        }
    }
}

/// Inner attributes `#![..]` of a file, module or block: they belong to it, not to the first item.
pub(crate) fn inner_attrs(p: &mut Parser<'_>) {
    while p.at(T::Hash) && p.nth_at(1, T::Bang) && p.nth_at(2, T::LBracket) {
        let m = p.start();
        p.bump_n(2);
        token_tree(p);
        m.complete(p, K::Attr);
    }
}

/// Outer and inner attributes: `#[..]`, `#![..]`.
pub(crate) fn attrs(p: &mut Parser<'_>) {
    while p.at(T::Hash) && (p.nth_at(1, T::LBracket) || (p.nth_at(1, T::Bang) && p.nth_at(2, T::LBracket))) {
        let m = p.start();
        p.bump();
        p.eat(T::Bang);
        token_tree(p);
        m.complete(p, K::Attr);
    }
}

fn visibility(p: &mut Parser<'_>) {
    if !p.at(T::Pub) {
        return;
    }
    let m = p.start();
    p.bump();
    if p.at(T::LParen) && matches!(p.nth(1), Some(T::Crate | T::SelfKw | T::Super | T::In)) {
        token_tree(p);
    }
    m.complete(p, K::Visibility);
}

/// Whether an item starts here (used inside blocks, where an expression is the alternative).
pub(crate) fn at_item_start(p: &mut Parser<'_>) -> bool {
    match p.current() {
        Some(T::Fn | T::Struct | T::Enum | T::Trait | T::Impl | T::Mod | T::Use | T::Pub | T::Static | T::Type | T::Extern) => true,
        Some(T::Const) => !p.nth_at(1, T::LBrace),
        Some(T::Unsafe) => matches!(p.nth(1), Some(T::Fn | T::Impl | T::Trait | T::Extern)),
        Some(T::Async) => p.nth_at(1, T::Fn) || (p.nth_at(1, T::Unsafe) && p.nth_at(2, T::Fn)),
        Some(T::Ident) => match p.nth(1) {
            Some(T::Ident) => p.nth_text_eq(0, "union") || p.nth_text_eq(0, "macro"),
            Some(T::Bang) => p.nth_text_eq(0, "macro_rules"),
            Some(T::Trait) => p.nth_text_eq(0, "auto"),
            _ => false,
        },
        _ => false,
    }
}

/// One item with its attributes and visibility; `None` (nothing consumed but attributes) when none starts here.
pub(crate) fn item(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    let start = p.mark_pos();
    attrs(p);
    let after_attrs = p.mark_pos();
    visibility(p);
    // Modifiers in front of `fn`, `impl`, `trait`.
    loop {
        match p.current() {
            Some(T::Unsafe | T::Async) if !p.nth_at(1, T::LBrace) && !(p.at(T::Async) && p.nth_at(1, T::Move)) => p.bump(),
            Some(T::Const) if matches!(p.nth(1), Some(T::Fn | T::Unsafe | T::Async | T::Extern | T::Trait | T::Impl)) => p.bump(),
            Some(T::Extern) if matches!(p.nth(1), Some(T::Fn)) || (p.nth(1).is_some_and(|k| k.is_literal()) && p.nth_at(2, T::Fn)) => {
                let a = p.start();
                p.bump();
                if p.current().is_some_and(|k| k.is_literal()) {
                    p.bump();
                }
                a.complete(p, K::Abi);
            }
            Some(T::Ident)
                if matches!(p.nth(1), Some(T::Fn | T::Impl | T::Unsafe | T::Async | T::Const | T::Type | T::Trait | T::Extern | T::Static))
                    && matches!(p.nth_text(0), "default" | "safe" | "auto") =>
            {
                p.bump()
            }
            _ => break,
        }
    }
    let kind = match p.current() {
        Some(T::Fn) => fn_(p),
        Some(T::Impl) => impl_(p),
        Some(T::Trait) => trait_(p),
        Some(T::Struct) => adt(p, K::Struct),
        Some(T::Enum) => adt(p, K::Enum),
        Some(T::Ident) if p.nth_text(0) == "union" && p.nth_at(1, T::Ident) => adt(p, K::Union),
        Some(T::Type) => type_alias(p),
        Some(T::Const) => const_or_static(p, K::Const),
        Some(T::Static) => const_or_static(p, K::Static),
        Some(T::Mod) => module(p),
        Some(T::Use) => to_semi(p, K::Use),
        Some(T::Extern) if p.nth_at(1, T::Crate) => to_semi(p, K::ExternCrate),
        Some(T::Extern) => extern_block(p),
        Some(T::Ident) if p.nth_text(0) == "macro_rules" && p.nth_at(1, T::Bang) => macro_rules(p),
        Some(T::Ident) if p.nth_text(0) == "macro" && p.nth_at(1, T::Ident) => {
            // Declarative macros 2.0: `macro name($x:expr) { .. }` or `macro name { (..) => {..} }`.
            p.bump();
            name(p);
            while matches!(p.current(), Some(T::LParen | T::LBrace)) {
                token_tree(p);
            }
            K::MacroRules
        }
        Some(T::Ident | T::SelfKw | T::Super | T::Crate) if macro_call_ahead(p) => macro_call_item(p),
        _ => {
            // Nothing, or only attributes (a file of inner `#![..]`): they stay with the enclosing node.
            if p.mark_pos() == start || p.mark_pos() == after_attrs {
                m.abandon(p);
                return None;
            }
            K::Error
        }
    };
    Some(m.complete(p, kind))
}

/// Whether a `path!` macro invocation starts here.
fn macro_call_ahead(p: &mut Parser<'_>) -> bool {
    let mut i = 0;
    loop {
        if !matches!(p.nth(i), Some(T::Ident | T::RawIdent | T::SelfKw | T::Super | T::Crate)) {
            return false;
        }
        i += 1;
        if p.nth_at(i, T::Bang) {
            return true;
        }
        if p.nth_at(i, T::Colon) && p.nth_at(i + 1, T::Colon) {
            i += 2;
        } else {
            return false;
        }
    }
}

fn macro_call_item(p: &mut Parser<'_>) -> K {
    types::path(p, types::PathMode::Expr);
    p.expect(T::Bang);
    let brace = p.at(T::LBrace);
    token_tree(p);
    if !brace {
        p.eat(T::Semi);
    }
    K::MacroCall
}

fn macro_rules(p: &mut Parser<'_>) -> K {
    p.bump();
    p.bump();
    name(p);
    let brace = p.at(T::LBrace);
    token_tree(p);
    if !brace {
        p.eat(T::Semi);
    }
    K::MacroRules
}

fn impl_(p: &mut Parser<'_>) -> K {
    let impl_start = p.tok(0).map_or(0, |t| t.start);
    p.bump();
    if p.at(T::Lt) {
        types::opt_generic_params(p);
    }
    p.eat(T::Const);
    p.eat(T::Bang);
    types::type_(p);
    if p.eat(T::For) {
        // `impl Display for dyn Expected + '_`.
        types::type_(p);
    }
    types::opt_where_clause(p);
    if !p.at(T::LBrace) || !super::items_fn::try_skip_brace_node(p, impl_start, K::ItemList) {
        assoc_items(p);
    }
    K::Impl
}

fn trait_(p: &mut Parser<'_>) -> K {
    let trait_start = p.tok(0).map_or(0, |t| t.start);
    p.bump();
    name(p);
    types::opt_generic_params(p);
    if p.eat(T::Colon) {
        types::bounds(p);
    }
    if p.eat(T::Eq) {
        types::bounds(p);
        types::opt_where_clause(p);
        p.expect(T::Semi);
        return K::Trait;
    }
    types::opt_where_clause(p);
    if !p.at(T::LBrace) || !super::items_fn::try_skip_brace_node(p, trait_start, K::ItemList) {
        assoc_items(p);
    }
    K::Trait
}

/// `{ items }` of an impl, trait, module or extern block.
fn assoc_items(p: &mut Parser<'_>) {
    if !p.at(T::LBrace) {
        p.expect(T::LBrace);
        return;
    }
    let m = p.start();
    p.bump();
    item_list(p, true);
    p.expect(T::RBrace);
    m.complete(p, K::ItemList);
}

/// `struct`, `enum`, `union`: header parsed, fields kept as a token tree.
fn adt(p: &mut Parser<'_>, kind: K) -> K {
    p.bump();
    name(p);
    types::opt_generic_params(p);
    types::opt_where_clause(p);
    match p.current() {
        Some(T::LBrace) => token_tree(p),
        Some(T::LParen) => {
            token_tree(p);
            types::opt_where_clause(p);
            p.expect(T::Semi);
        }
        _ => {
            p.expect(T::Semi);
        }
    }
    kind
}

fn type_alias(p: &mut Parser<'_>) -> K {
    p.bump();
    name(p);
    types::opt_generic_params(p);
    if p.eat(T::Colon) {
        types::bounds(p);
    }
    types::opt_where_clause(p);
    if p.eat(T::Eq) {
        types::type_(p);
    }
    types::opt_where_clause(p);
    p.expect(T::Semi);
    K::TypeAlias
}

fn const_or_static(p: &mut Parser<'_>, kind: K) -> K {
    p.bump();
    p.eat(T::Mut);
    if !p.eat(T::Underscore) {
        name(p);
    }
    types::opt_generic_params(p);
    if p.eat(T::Colon) {
        types::type_(p);
    }
    if p.eat(T::Eq) {
        super::expr::expr(p);
    }
    types::opt_where_clause(p);
    p.expect(T::Semi);
    kind
}

fn module(p: &mut Parser<'_>) -> K {
    let mod_start = p.tok(0).map_or(0, |t| t.start);
    p.bump();
    name(p);
    if p.at(T::LBrace) {
        if !super::items_fn::try_skip_mod_item_list(p, mod_start) {
            assoc_items(p);
        }
    } else {
        p.expect(T::Semi);
    }
    K::Module
}

fn extern_block(p: &mut Parser<'_>) -> K {
    let a = p.start();
    p.bump();
    if p.current().is_some_and(|k| k.is_literal()) {
        p.bump();
    }
    a.complete(p, K::Abi);
    assoc_items(p);
    K::ExternBlock
}

/// Items kept as tokens up to their `;` (`use` trees, `extern crate`).
fn to_semi(p: &mut Parser<'_>, kind: K) -> K {
    while !p.at_eof() && !p.at(T::Semi) && !p.at(T::RBrace) {
        if p.at(T::LBrace) {
            token_tree(p);
        } else {
            p.bump();
        }
    }
    p.expect(T::Semi);
    kind
}
