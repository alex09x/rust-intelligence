/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! What decides the postfix snippets after `receiver.`: facts about the receiver's type and the syntactic position
//! of the `receiver.name` expression, following rust-analyzer's postfix rules.

use ridx_syntax::name_res::{ImplTargetKey, PrimTy};
use ridx_syntax::ty_infer::autoderef::impl_target_key_of_ty;
use ridx_syntax::ty_infer::{InferTyArena, InferTyId, InferTyKind, TyDefDb, TyLangItem};
use ridx_syntax::{NodeId, SyntaxKind as K, SyntaxTree, TokenKind as T};

use super::query::Cand;
use super::v5_db::RidxV5ItemDb;

/// `Option` or `Result`: the receivers that get `ifl`, `lete` and `while let` instead of `for`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TryEnum {
    Option,
    Result,
}

/// The receiver's type and the position of `receiver.name` that the postfix snippet set depends on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PostfixFacts {
    pub try_enum: Option<TryEnum>,
    pub is_bool: bool,
    pub into_iter: bool,
    pub impls_drop: bool,
    /// The condition of an `if` / `while` or a match guard, or an operand of a `&&` chain that is one.
    pub in_condition: bool,
    /// A statement or the tail expression of its block.
    pub in_block: bool,
    /// The whole expression is a match arm's body or a closure's body.
    pub in_arm_or_closure: bool,
    /// Inside the body of a `loop` or a labeled block of the same function or closure.
    pub in_breakable: bool,
}

/// The facts for the dot whose receiver ends at `dot_end - 1`. Inside a macro call's token tree, where no
/// `receiver.name` expression is in the tree, only the receiver's type facts are set.
pub(crate) fn postfix_facts(st: &SyntaxTree<'_>, dot_end: u32, db: &RidxV5ItemDb<'_>, arena: &InferTyArena, recv_ty: InferTyId, cands: &[Cand]) -> Option<PostfixFacts> {
    let dot = dot_end.checked_sub(1)? as usize;
    let cov = st.covering(dot);
    let expr = std::iter::once(cov)
        .chain(st.ancestors(cov))
        .find(|&n| matches!(st.kind(n), K::MethodCallExpr | K::FieldExpr) && st.children(n).next().is_some_and(|r| st.range(r).end <= dot));
    let with_refs = expr.map(|e| {
        let mut n = e;
        while let Some(p) = st.parent(n).filter(|&p| matches!(st.kind(p), K::RefExpr | K::PrefixExpr)) {
            n = p;
        }
        n
    });
    let mut stripped = recv_ty;
    while let InferTyKind::Ref { inner, .. } = arena.kind(stripped) {
        stripped = inner;
    }
    let try_enum = match arena.kind(stripped) {
        InferTyKind::Adt { def, .. } => match db.ty_tables.gid_to_lang.get(&db.canonical_def(def)) {
            Some(TyLangItem::Option) => Some(TryEnum::Option),
            Some(TyLangItem::Result) => Some(TryEnum::Result),
            _ => None,
        },
        _ => None,
    };
    let impls_drop = db.lang_item(TyLangItem::Drop).is_some_and(|d| {
        let head = match impl_target_key_of_ty(arena, recv_ty) {
            ImplTargetKey::Adt(a) => ImplTargetKey::Adt(db.canonical_def(a)),
            other => other,
        };
        head != ImplTargetKey::Unknown && !db.ty_tables.concrete_impls(&(db.canonical_def(d), head)).is_empty()
    });
    Some(PostfixFacts {
        try_enum,
        is_bool: matches!(arena.kind(stripped), InferTyKind::Prim(PrimTy::Bool)),
        into_iter: cands.iter().any(|c| c.label == "into_iter" && c.from_trait.as_deref().is_some_and(|t| t.ends_with("IntoIterator"))),
        impls_drop,
        in_condition: with_refs.is_some_and(|n| is_in_condition(st, n)),
        in_block: expr.is_some_and(|e| is_in_block(st, e)),
        in_arm_or_closure: with_refs.and_then(|n| st.parent(n)).is_some_and(|p| matches!(st.kind(p), K::MatchArm | K::ClosureExpr)),
        in_breakable: expr.is_some_and(|e| is_in_breakable(st, e)),
    })
}

/// The first child of `n` that is not an attribute or a label: the condition of an `if` / `while`.
fn first_operand(st: &SyntaxTree<'_>, n: NodeId) -> Option<NodeId> {
    st.children(n).find(|&c| !matches!(st.kind(c), K::Attr | K::Label))
}

/// Whether the operator of `bin` is `&&` (lexed as two adjoining `&`).
fn is_and_chain(st: &SyntaxTree<'_>, bin: NodeId) -> bool {
    let mut prev_amp_end = None;
    for (_, t) in st.own_tokens(bin) {
        if t.kind == T::Amp {
            if prev_amp_end == Some(t.start) {
                return true;
            }
            prev_amp_end = Some(t.end);
        } else {
            prev_amp_end = None;
        }
    }
    false
}

fn is_in_condition(st: &SyntaxTree<'_>, mut n: NodeId) -> bool {
    while let Some(p) = st.parent(n) {
        match st.kind(p) {
            K::IfExpr | K::WhileExpr => return first_operand(st, p) == Some(n),
            K::MatchGuard => return true,
            K::BinExpr if is_and_chain(st, p) => n = p,
            _ => return false,
        }
    }
    false
}

fn is_in_block(st: &SyntaxTree<'_>, expr: NodeId) -> bool {
    st.parent(expr).is_some_and(|p| matches!(st.kind(p), K::ExprStmt | K::BlockExpr))
}

fn is_in_breakable(st: &SyntaxTree<'_>, expr: NodeId) -> bool {
    let at = st.range(expr).start;
    for a in st.ancestors(expr) {
        match st.kind(a) {
            K::Fn | K::ClosureExpr => return false,
            K::LoopExpr => {
                if st.child(a, K::BlockExpr).is_some_and(|b| st.range(b).contains(&at)) {
                    return true;
                }
            }
            K::BlockExpr if st.child(a, K::Label).is_some() => return true,
            _ => {}
        }
    }
    false
}
