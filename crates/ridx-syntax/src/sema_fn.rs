/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Syntax-only function-body semantics over [`SyntaxTree`]: parameter descriptors, scope-aware
//! parameter use analysis (with shadowing and macro `TokenTree` separation), own `return` / `?`
//! extraction, and tail expression lookup.

use std::ops::Range;

use crate::sema_macro::collect_own_cf;
pub use crate::sema_pat::ParamBindings;
use crate::sema_pat::{collect_pat_bindings, pat_binds_bytes};
use crate::sema_scope::find_param_uses;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;
use crate::tree::{NodeId, SyntaxTree};

/// Receiver form of a `self` parameter (`SelfParam`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelfKind<'s> {
    /// `self`
    Value,
    /// `mut self`
    MutValue,
    /// `&self` or `&'a self`
    Ref { lifetime: Option<&'s str> },
    /// `&mut self` or `&'a mut self`
    MutRef { lifetime: Option<&'s str> },
    /// Explicit receiver type: `self: T` or `mut self: T` (e.g. `self: Box<Self>`).
    Typed { mutability: bool, ty: &'s str },
}

impl<'s> SelfKind<'s> {
    pub const fn is_ref(self) -> bool {
        matches!(self, Self::Ref { .. } | Self::MutRef { .. })
    }

    pub const fn is_mut(self) -> bool {
        matches!(self, Self::MutValue | Self::MutRef { .. } | Self::Typed { mutability: true, .. })
    }

    pub const fn lifetime(self) -> Option<&'s str> {
        match self {
            Self::Ref { lifetime } | Self::MutRef { lifetime } => lifetime,
            _ => None,
        }
    }
}

/// Extracted syntax descriptor of one parameter (`SelfParam` or `Param`) in a `Fn` item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParamInfo<'s> {
    pub node: NodeId,
    pub self_kind: Option<SelfKind<'s>>,
    pub pat: Option<NodeId>,
    pub pat_text: Option<&'s str>,
    pub bindings: ParamBindings<'s>,
    pub ty: Option<NodeId>,
    pub ty_text: Option<&'s str>,
}

/// Uses of a function parameter inside the function body, split into confident AST references and
/// uncertain macro `TokenTree` occurrences.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParamUses {
    pub uses: Vec<Range<usize>>,
    pub macro_uses: Vec<Range<usize>>,
}

impl ParamUses {
    #[inline]
    pub fn is_unused(&self) -> bool {
        self.uses.is_empty() && self.macro_uses.is_empty()
    }

    #[inline]
    pub fn clear(&mut self) {
        self.uses.clear();
        self.macro_uses.clear();
    }
}

/// Strips a leading `r#` raw-identifier prefix so `r#foo` and `foo` compare equal.
#[inline]
pub fn norm_ident(raw: &str) -> &str {
    raw.strip_prefix("r#").unwrap_or(raw)
}

/// Returns the body `BlockExpr` of a `Fn` node, or `None` for declarations without a body (`fn f();`).
#[inline]
pub fn fn_body(tree: &SyntaxTree<'_>, fn_id: NodeId) -> Option<NodeId> {
    if tree.kind(fn_id) != K::Fn {
        return None;
    }
    // Body `BlockExpr` is always the last child of `Fn` when present.
    let last = tree.children(fn_id).last()?;
    (tree.kind(last) == K::BlockExpr).then_some(last)
}

/// Extracts all parameters (`SelfParam` followed by `Param`s) of `fn_id` into `out` (cleared first).
pub fn params_into<'s>(tree: &SyntaxTree<'s>, fn_id: NodeId, out: &mut Vec<ParamInfo<'s>>) {
    out.clear();
    if tree.kind(fn_id) != K::Fn {
        return;
    }
    let Some(param_list) = tree.child(fn_id, K::ParamList) else { return };
    let count = tree.children(param_list).filter(|&c| matches!(tree.kind(c), K::SelfParam | K::Param)).count();
    if count == 0 {
        return;
    }
    out.reserve(count);
    for ch in tree.children(param_list) {
        match tree.kind(ch) {
            K::SelfParam => out.push(extract_self_param(tree, ch)),
            K::Param => out.push(extract_value_param(tree, ch)),
            _ => {}
        }
    }
}

/// Extracts all parameters (`SelfParam` followed by `Param`s) of `fn_id` in declaration order.
pub fn params<'s>(tree: &SyntaxTree<'s>, fn_id: NodeId) -> Vec<ParamInfo<'s>> {
    let mut out = Vec::new();
    params_into(tree, fn_id, &mut out);
    out
}

/// Extracts the normalized bound identifier names introduced by `pat_id` in source order (deduplicated).
pub fn pat_bindings<'s>(tree: &SyntaxTree<'s>, pat_id: NodeId) -> Vec<&'s str> {
    let mut b = ParamBindings::empty();
    collect_pat_bindings(tree, pat_id, &mut b);
    b.as_slice().to_vec()
}

/// Finds uses of parameter `name` within `fn_id`'s body into reusable `out` (cleared first).
pub fn param_uses_into(tree: &SyntaxTree<'_>, fn_id: NodeId, name: &str, out: &mut ParamUses) {
    out.clear();
    let target = norm_ident(name);
    if target.is_empty() || target == "_" {
        return;
    }
    let Some(body) = fn_body(tree, fn_id) else { return };
    if !fn_declares_param(tree, fn_id, target) {
        return;
    }
    find_param_uses(tree, body, target, out);
}

/// Finds uses of parameter `name` (normalized; `"self"` for receiver) within `fn_id`'s body.
pub fn param_uses(tree: &SyntaxTree<'_>, fn_id: NodeId, name: &str) -> ParamUses {
    let mut out = ParamUses::default();
    param_uses_into(tree, fn_id, name, &mut out);
    out
}

/// Finds the uses of `name` (normalized), bound by the `let` statement `let_stmt`, in the rest of its block: up to a
/// statement that binds the name again, through blocks, closures and macro arguments, as `param_uses` does.
pub fn let_uses(tree: &SyntaxTree<'_>, let_stmt: NodeId, name: &str) -> ParamUses {
    let mut out = ParamUses::default();
    crate::sema_scope::find_let_uses(tree, let_stmt, name, &mut out);
    out
}

/// Collects `ReturnExpr` nodes belonging to `fn_id` into `out` (cleared first).
pub fn own_returns_into(tree: &SyntaxTree<'_>, fn_id: NodeId, out: &mut Vec<NodeId>) {
    out.clear();
    if let Some(body) = fn_body(tree, fn_id) {
        collect_own_cf(tree, body, false, K::ReturnExpr, out);
    }
}

/// Returns all `ReturnExpr` nodes belonging to `fn_id` itself.
pub fn own_returns(tree: &SyntaxTree<'_>, fn_id: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    own_returns_into(tree, fn_id, &mut out);
    out
}

/// Collects `TryExpr` (`?`) nodes belonging to `fn_id` into `out` (cleared first).
pub fn own_try_ops_into(tree: &SyntaxTree<'_>, fn_id: NodeId, out: &mut Vec<NodeId>) {
    out.clear();
    if let Some(body) = fn_body(tree, fn_id) {
        collect_own_cf(tree, body, true, K::TryExpr, out);
    }
}

/// Returns all `TryExpr` (`?`) nodes belonging to `fn_id` itself.
pub fn own_try_ops(tree: &SyntaxTree<'_>, fn_id: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    own_try_ops_into(tree, fn_id, &mut out);
    out
}

/// Returns the tail expression of `fn_id`'s body `BlockExpr`, if present.
#[inline]
pub fn tail_expr(tree: &SyntaxTree<'_>, fn_id: NodeId) -> Option<NodeId> {
    let body = fn_body(tree, fn_id)?;
    let last = tree.children(body).last()?;
    let k = tree.kind(last);
    (k.is_expr() || k == K::MacroCall).then_some(last)
}

fn fn_declares_param(tree: &SyntaxTree<'_>, fn_id: NodeId, target: &str) -> bool {
    let Some(pl) = tree.child(fn_id, K::ParamList) else { return false };
    if target == "self" {
        return tree.children(pl).next().is_some_and(|c| tree.kind(c) == K::SelfParam);
    }
    let tb = target.as_bytes();
    let src = tree.src.as_bytes();
    for ch in tree.children(pl) {
        if tree.kind(ch) == K::Param {
            if let Some(pat) = tree.children(ch).find(|&c| tree.kind(c).is_pat()) {
                if pat_binds_bytes(&tree.nodes, &tree.tokens, src, pat, tb) {
                    return true;
                }
            }
        }
    }
    false
}

fn extract_self_param<'s>(tree: &SyntaxTree<'s>, id: NodeId) -> ParamInfo<'s> {
    let mut has_amp = false;
    let mut has_mut = false;
    for tok in tree.tokens_of(id) {
        match tok.kind {
            T::Amp => has_amp = true,
            T::Mut => has_mut = true,
            T::SelfKw => break,
            _ => {}
        }
    }
    let lifetime = tree.child(id, K::Lifetime).map(|lt| tree.text(lt));
    let ty = tree.children(id).find(|&c| tree.kind(c).is_type() || tree.kind(c) == K::MacroCall);
    let ty_text = ty.map(|t| tree.text(t));
    let self_kind = if let Some(t_str) = ty_text {
        SelfKind::Typed { mutability: has_mut, ty: t_str }
    } else if has_amp {
        if has_mut {
            SelfKind::MutRef { lifetime }
        } else {
            SelfKind::Ref { lifetime }
        }
    } else if has_mut {
        SelfKind::MutValue
    } else {
        SelfKind::Value
    };
    ParamInfo { node: id, self_kind: Some(self_kind), pat: None, pat_text: None, bindings: ParamBindings::single("self"), ty, ty_text }
}

fn extract_value_param<'s>(tree: &SyntaxTree<'s>, id: NodeId) -> ParamInfo<'s> {
    let mut pat = None;
    let mut ty = None;
    for ch in tree.children(id) {
        let ck = tree.kind(ch);
        if pat.is_none() && ck.is_pat() {
            pat = Some(ch);
        } else if ck.is_type() || ck == K::MacroCall {
            ty = Some(ch);
        }
    }
    let mut bindings = ParamBindings::empty();
    if let Some(p) = pat {
        collect_pat_bindings(tree, p, &mut bindings);
    }
    ParamInfo { node: id, self_kind: None, pat, pat_text: pat.map(|p| tree.text(p)), bindings, ty, ty_text: ty.map(|t| tree.text(t)) }
}
