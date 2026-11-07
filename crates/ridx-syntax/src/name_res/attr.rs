/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Zero-panic attribute evaluation over [`SyntaxTree`] `Attr` nodes: `#[cfg(...)]`,
//! `#![cfg(...)]`, recursive `#[cfg_attr(pred, ...)]`, `#[path = "..."]`, `#[macro_use]`,
//! `#[macro_export]`, `#![no_std]`, and `#![no_implicit_prelude]`.

use std::borrow::Cow;

use super::cfg::{decode_str_literal, eval_cfg_tokens};
use super::db::{CrateId, ItemDb, VisKind};
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::{Token, TokenKind as T};
use crate::tree::{NodeId, SyntaxTree};

/// Evaluated summary of effective attributes on an item or module container.
#[derive(Clone, Debug)]
pub struct EffectiveAttrs<'s> {
    /// `false` when at least one active `cfg(...)` predicate evaluated to `false`.
    pub enabled: bool,
    /// First effective `#[path = "..."]` string value, if any.
    pub explicit_path: Option<Cow<'s, str>>,
    /// `#[macro_use]` or `#[macro_use(name1, name2)]` was active.
    pub macro_use: bool,
    /// If `#[macro_use(a, b)]` listed explicit names, restricts upward export to these names.
    pub macro_use_filter: Option<Vec<&'s str>>,
    /// `#[macro_export]` was active on a `macro_rules!` definition.
    pub macro_export: bool,
    /// `#[no_implicit_prelude]` / `#![no_implicit_prelude]` was active.
    pub no_implicit_prelude: bool,
    /// `#![no_std]` was active.
    pub no_std: bool,
}

impl Default for EffectiveAttrs<'_> {
    #[inline]
    fn default() -> Self {
        Self { enabled: true, explicit_path: None, macro_use: false, macro_use_filter: None, macro_export: false, no_implicit_prelude: false, no_std: false }
    }
}

/// Evaluates all outer `#[...]` attributes attached as direct `Attr` children of `item_node`.
#[inline]
pub fn eval_outer_attrs<'s>(tree: &SyntaxTree<'s>, item_node: NodeId, db: &dyn ItemDb, krate: CrateId) -> EffectiveAttrs<'s> {
    let mut out = EffectiveAttrs::default();
    eval_attrs_on_container(tree, item_node, false, db, krate, &mut out);
    out
}

/// Evaluates all inner `#![...]` attributes attached directly inside `container_node`
/// (`SourceFile` for file modules, or `ItemList` for inline `mod foo { #![...] }`).
#[inline]
pub fn eval_inner_attrs<'s>(tree: &SyntaxTree<'s>, container_node: NodeId, db: &dyn ItemDb, krate: CrateId, out: &mut EffectiveAttrs<'s>) {
    eval_attrs_on_container(tree, container_node, true, db, krate, out);
}

fn eval_attrs_on_container<'s>(tree: &SyntaxTree<'s>, container: NodeId, want_inner: bool, db: &dyn ItemDb, krate: CrateId, out: &mut EffectiveAttrs<'s>) {
    for child in tree.children(container) {
        if tree.kind(child) != K::Attr {
            if want_inner {
                // Inner attributes always precede items in `SourceFile` / `ItemList`.
                break;
            }
            continue;
        }
        let toks = tree.tokens_of(child);
        let is_inner = is_inner_attr_tokens(toks);
        if is_inner != want_inner {
            continue;
        }
        if let Some(inner_slice) = bracket_inner_tokens(toks) {
            eval_meta_item(tree.src, inner_slice, db, krate, out, 0);
            if !out.enabled {
                // Short-circuit once disabled by `cfg(...)`.
                return;
            }
        }
    }
}

const MAX_CFG_ATTR_DEPTH: u32 = 16;

fn eval_meta_item<'s>(src: &'s str, tokens: &[Token], db: &dyn ItemDb, krate: CrateId, out: &mut EffectiveAttrs<'s>, depth: u32) {
    if depth >= MAX_CFG_ATTR_DEPTH || !out.enabled {
        return;
    }
    // Extract leading attribute path (`cfg`, `core::prelude::v1::cfg`, `cfg_attr`, `path`, ...).
    let (attr_name, rest) = match split_attr_head(src, tokens) {
        Some(pair) => pair,
        None => return,
    };
    match attr_name {
        "cfg" => {
            if let Some(paren_inner) = paren_inner_tokens(rest) {
                if !eval_cfg_tokens(src, paren_inner, db, krate) {
                    out.enabled = false;
                }
            }
        }
        "cfg_attr" => {
            let Some(paren_inner) = paren_inner_tokens(rest) else {
                return;
            };
            let Some((pred_slice, after_comma)) = split_top_level_comma(paren_inner) else {
                return;
            };
            if eval_cfg_tokens(src, pred_slice, db, krate) {
                let mut rem = after_comma;
                while !rem.is_empty() {
                    let (meta_slice, next_rem) = match split_top_level_comma(rem) {
                        Some((a, b)) => (a, b),
                        None => (rem, &[][..]),
                    };
                    if has_non_trivia(meta_slice) {
                        eval_meta_item(src, meta_slice, db, krate, out, depth + 1);
                        if !out.enabled {
                            return;
                        }
                    }
                    rem = next_rem;
                }
            }
        }
        "path" => {
            if out.explicit_path.is_none() {
                if let Some(val_tok) = eq_value_token(rest) {
                    out.explicit_path = decode_str_literal(src, val_tok);
                }
            }
        }
        "macro_use" => {
            out.macro_use = true;
            if let Some(paren_inner) = paren_inner_tokens(rest) {
                let list = out.macro_use_filter.get_or_insert_with(Vec::new);
                for tok in paren_inner {
                    if matches!(tok.kind, T::Ident | T::RawIdent) {
                        list.push(norm_ident(tok.text(src)));
                    }
                }
            }
        }
        "macro_export" => {
            out.macro_export = true;
        }
        "no_implicit_prelude" => {
            out.no_implicit_prelude = true;
        }
        "no_std" => {
            out.no_std = true;
        }
        _ => {}
    }
}

/// Extracts visibility of an item node (`Visibility` child -> `VisKind`, plus optional `in path` text).
pub fn extract_visibility<'s>(tree: &SyntaxTree<'s>, item_node: NodeId) -> (VisKind, Option<&'s str>) {
    let Some(vis_node) = tree.child(item_node, K::Visibility) else {
        return (VisKind::Private, None);
    };
    let toks = tree.tokens_of(vis_node);
    let Some(paren_inner) = paren_inner_tokens(toks) else {
        return (VisKind::Public, None);
    };
    let mut sig = paren_inner.iter().filter(|t| !t.is_trivia());
    let Some(first) = sig.next() else {
        return (VisKind::Public, None);
    };
    match first.kind {
        T::Crate => (VisKind::Crate, None),
        T::SelfKw => (VisKind::Private, None),
        T::Super => {
            if sig.next().is_none() {
                (VisKind::Super, None)
            } else {
                let start = first.start as usize;
                let end = paren_inner.iter().rfind(|t| !t.is_trivia()).map_or(first.end as usize, |t| t.end as usize);
                (VisKind::Restricted, Some(&tree.src[start..end]))
            }
        }
        T::In => {
            let Some(path_first) = sig.next() else {
                return (VisKind::Private, None);
            };
            let end = paren_inner.iter().rfind(|t| !t.is_trivia()).map_or(path_first.end as usize, |t| t.end as usize);
            (VisKind::Restricted, Some(&tree.src[path_first.start as usize..end]))
        }
        _ => (VisKind::Restricted, None),
    }
}

fn is_inner_attr_tokens(toks: &[Token]) -> bool {
    let mut sig = toks.iter().filter(|t| !t.is_trivia());
    matches!(sig.next().map(|t| t.kind), Some(T::Hash)) && matches!(sig.next().map(|t| t.kind), Some(T::Bang))
}

fn bracket_inner_tokens(toks: &[Token]) -> Option<&[Token]> {
    let open = toks.iter().position(|t| t.kind == T::LBracket)?;
    let close = toks.iter().rposition(|t| t.kind == T::RBracket);
    match close {
        Some(c) if c > open => Some(&toks[open + 1..c]),
        _ => Some(&toks[open + 1..]),
    }
}

fn paren_inner_tokens(toks: &[Token]) -> Option<&[Token]> {
    let open = toks.iter().position(|t| t.kind == T::LParen)?;
    let mut depth = 0u32;
    for (idx, t) in toks[open..].iter().enumerate() {
        match t.kind {
            T::LParen => depth += 1,
            T::RParen => {
                depth -= 1;
                if depth == 0 {
                    return Some(&toks[open + 1..open + idx]);
                }
            }
            _ => {}
        }
    }
    Some(&toks[open + 1..])
}

fn split_attr_head<'s, 't>(src: &'s str, toks: &'t [Token]) -> Option<(&'s str, &'t [Token])> {
    let mut i = 0;
    let mut last_seg: Option<&'s str> = None;
    loop {
        while i < toks.len() && toks[i].is_trivia() {
            i += 1;
        }
        if i >= toks.len() {
            break;
        }
        // Optional leading `::`
        if last_seg.is_none() && toks[i].kind == T::Colon && toks.get(i + 1).map(|t| t.kind) == Some(T::Colon) {
            i += 2;
            continue;
        }
        let tk = toks[i].kind;
        if matches!(tk, T::Ident | T::RawIdent) || tk.is_keyword() {
            last_seg = Some(norm_ident(toks[i].text(src)));
            i += 1;
            let mut j = i;
            while j < toks.len() && toks[j].is_trivia() {
                j += 1;
            }
            if j + 1 < toks.len() && toks[j].kind == T::Colon && toks[j + 1].kind == T::Colon {
                i = j + 2;
                continue;
            }
        }
        break;
    }
    Some((last_seg?, &toks[i..]))
}

fn split_top_level_comma(toks: &[Token]) -> Option<(&[Token], &[Token])> {
    let mut depth = 0u32;
    for (i, t) in toks.iter().enumerate() {
        match t.kind {
            T::LParen | T::LBracket | T::LBrace => depth += 1,
            T::RParen | T::RBracket | T::RBrace => depth = depth.saturating_sub(1),
            T::Comma if depth == 0 => return Some((&toks[..i], &toks[i + 1..])),
            _ => {}
        }
    }
    None
}

fn eq_value_token(toks: &[Token]) -> Option<Token> {
    let mut sig = toks.iter().filter(|t| !t.is_trivia());
    if sig.next()?.kind != T::Eq {
        return None;
    }
    sig.next().copied()
}

fn has_non_trivia(toks: &[Token]) -> bool {
    toks.iter().any(|t| !t.is_trivia())
}
