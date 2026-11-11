/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Ordered dot-completion method collector (`BodyInferCtx::collect_ordered_completion_methods`)
//! preserving exact `method_res` autoderef step and `inherent -> concrete trait -> blanket trait` tiers.

use super::autoderef::{build_autoderef_steps_with_owner, MAX_AUTODEREF_STEPS};
use super::body_ctx::{BodyInferCtx, FnInferResult, Runner};
use super::db::TyDefDb;
use super::local_defs::LocalCstDefs;
use super::ty::{InferTyId, InferTyKind};
use crate::name_res::{FileNameRes, GlobalDefId};
use crate::syntax_kind::SyntaxKind as K;
use crate::tree::{NodeId, SyntaxTree};

/// Matched dot-completion method candidate with autoderef step and `method_res` precedence tier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompletionMethodHit {
    pub step_idx: u8,
    /// `0` = inherent, `1` = concrete/bound trait impl, `2` = blanket trait impl.
    pub tier: u8,
    pub name_idx: u32,
    pub method_def: GlobalDefId,
    pub trait_def: GlobalDefId,
}

/// Which scope traits declare each candidate name, so a name is probed only against the traits that have it.
///
/// `known` lists the scope traits whose full method-name set the caller has, sorted by `(krate, index)`;
/// `declaring[ranges[i].0..ranges[i].1]` are the ones among them that declare `candidate_names[i]`. A scope trait
/// outside `known` is probed for every name.
#[derive(Clone, Copy, Default)]
pub struct CompletionTraitHints<'a> {
    pub known: &'a [GlobalDefId],
    pub ranges: &'a [(u32, u32)],
    pub declaring: &'a [GlobalDefId],
}

impl BodyInferCtx {
    /// Collects all callable methods in `candidate_names` for `recv_ty` at `at_node` inside `fn_node`,
    /// also writing the computed autoderef `steps` into `steps_out` (`0` = direct receiver, `1..` = derefs).
    #[allow(clippy::too_many_arguments)]
    /// The traits bounding the type parameter `ty` inside `fn_node` (the enclosing trait for its `Self`, the bounds
    /// written in the source and the indexed ones, with supertraits), appended to `out`.
    pub fn param_bounds_for_completion(
        &mut self,
        st: &SyntaxTree<'_>,
        file_res: &FileNameRes,
        db: &dyn TyDefDb,
        local_defs: &LocalCstDefs,
        fn_node: NodeId,
        ty: InferTyId,
        out: &mut Vec<GlobalDefId>,
    ) {
        let snap = self.table.start_snapshot();
        let impl_node = st.ancestors(fn_node).find(|&a| matches!(st.kind(a), K::Impl | K::Trait));
        let (impl_gc, self_ty) = local_defs.owner_generics_and_self_ty(impl_node, st, file_res, db, &mut self.table);
        let fn_def = local_defs.fn_def_for_node(fn_node);
        let mut dummy_out = FnInferResult::new();
        let env = Runner {
            ctx: self,
            st,
            file_res,
            root_st: st,
            root_file_res: file_res,
            db,
            local_defs,
            fn_node,
            root_fn_node: fn_node,
            fn_def,
            impl_node,
            root_impl_node: impl_node,
            impl_gc,
            self_ty,
            loc_lo: 0,
            loc_hi: 0,
            expr_depth: 0,
            sub: None,
            out: &mut dummy_out,
        };
        let mut found = Vec::new();
        env.collect_ty_direct_bounds(ty, &mut found);
        for t in found {
            if !out.contains(&t) {
                out.push(t);
            }
        }
        self.table.rollback_to(snap);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn collect_ordered_completion_methods(
        &mut self,
        st: &SyntaxTree<'_>,
        file_res: &FileNameRes,
        db: &dyn TyDefDb,
        local_defs: &LocalCstDefs,
        fn_node: NodeId,
        at_node: NodeId,
        recv_ty: InferTyId,
        candidate_names: &[&str],
        hints: &CompletionTraitHints<'_>,
        steps_out: &mut Vec<InferTyId>,
        out: &mut Vec<CompletionMethodHit>,
    ) {
        steps_out.clear();
        out.clear();
        let top_snap = self.table.start_snapshot();
        self.fn_has_inner_trait_scopes = file_res.has_inner_trait_scopes(fn_node, st.node(fn_node).subtree_end);
        self.fn_scope_traits_ready = false;
        let impl_node = st.ancestors(fn_node).find(|&a| matches!(st.kind(a), K::Impl | K::Trait));
        let (impl_gc, self_ty) = local_defs.owner_generics_and_self_ty(impl_node, st, file_res, db, &mut self.table);
        let fn_def = local_defs.fn_def_for_node(fn_node);
        let mut dummy_out = FnInferResult::new();
        let mut env = Runner {
            ctx: self,
            st,
            file_res,
            root_st: st,
            root_file_res: file_res,
            db,
            local_defs,
            fn_node,
            root_fn_node: fn_node,
            fn_def,
            impl_node,
            root_impl_node: impl_node,
            impl_gc,
            self_ty,
            loc_lo: 0,
            loc_hi: 0,
            expr_depth: 0,
            sub: None,
            out: &mut dummy_out,
        };
        let mut steps = [InferTyId::UNKNOWN; MAX_AUTODEREF_STEPS];
        let owner = env.norm_owner();
        let step_len = build_autoderef_steps_with_owner(&mut env.ctx.table, db, local_defs, st, file_res, owner, recv_ty, &mut steps);
        steps_out.extend_from_slice(&steps[..step_len]);
        let mut cands = core::mem::take(&mut env.ctx.method_cands_buf);
        let mut scope_traits = core::mem::take(&mut env.ctx.scope_traits_buf);
        env.ctx.compl_known_traits.clear();
        env.ctx.compl_known_traits.extend_from_slice(hints.known);
        env.ctx.compl_exact_scope = true;
        for (n_idx, &mname) in candidate_names.iter().enumerate() {
            cands.clear();
            scope_traits.clear();
            env.ctx.compl_name_traits.clear();
            if let Some(&(lo, hi)) = hints.ranges.get(n_idx) {
                env.ctx.compl_name_traits.extend_from_slice(&hints.declaring[lo as usize..hi as usize]);
            }
            let inh_end = env.gather_inherent_cands(mname, &steps[..step_len], &mut cands);
            env.gather_trait_and_bound_cands(at_node, mname, &steps[..step_len], &mut scope_traits, &mut cands);
            for (step_idx, &st_ty) in steps[..step_len].iter().enumerate() {
                let mut rcvrs = [InferTyId::NONE; 7];
                let mut rlen = 0usize;
                rcvrs[rlen] = st_ty;
                rlen += 1;
                if let InferTyKind::Ref { is_mut: true, inner } = env.ctx.table.arena.kind(st_ty) {
                    rcvrs[rlen] = env.ctx.table.arena.mk_ref(false, inner);
                    rlen += 1;
                }
                rcvrs[rlen] = env.ctx.table.arena.mk_ref(false, st_ty);
                rlen += 1;
                rcvrs[rlen] = env.ctx.table.arena.mk_ref(true, st_ty);
                rlen += 1;
                if let InferTyKind::Array { elem, .. } = env.ctx.table.arena.kind(st_ty) {
                    let sl = env.ctx.table.arena.mk_slice(elem);
                    rcvrs[rlen] = sl;
                    rlen += 1;
                    rcvrs[rlen] = env.ctx.table.arena.mk_ref(false, sl);
                    rlen += 1;
                    rcvrs[rlen] = env.ctx.table.arena.mk_ref(true, sl);
                    rlen += 1;
                }
                for (ci, cand) in cands.iter().enumerate() {
                    if rcvrs[..rlen].iter().any(|&rcvr| env.cand_matches_completion_rcvr(cand, rcvr)) {
                        let tier = if ci < inh_end {
                            0
                        } else if !cand.is_blanket_impl {
                            1
                        } else {
                            2
                        };
                        out.push(CompletionMethodHit { step_idx: step_idx as u8, tier, name_idx: n_idx as u32, method_def: cand.method_def, trait_def: cand.trait_def });
                    }
                }
            }
        }
        env.ctx.compl_known_traits.clear();
        env.ctx.compl_name_traits.clear();
        env.ctx.compl_exact_scope = false;
        env.ctx.method_cands_buf = cands;
        env.ctx.scope_traits_buf = scope_traits;
        self.table.rollback_to(top_snap);
        out.sort_unstable_by_key(|h| (h.step_idx, h.tier, h.name_idx, h.method_def.krate.0, h.method_def.index));
    }
}

impl LocalCstDefs {
    /// Collects `(field_name, span_start, span_end)` for a CST `struct` / `union` item recorded in `self`.
    pub fn cst_struct_field_spans<'s>(&self, def: GlobalDefId, st: &'s SyntaxTree<'s>, out: &mut Vec<(String, u32, u32)>) {
        out.clear();
        let node_raw = self.fn_node_for_def(def);
        if node_raw == u32::MAX {
            return;
        }
        let item_node = NodeId(node_raw);
        let Some(tt) = st.child(item_node, K::TokenTree) else { return };
        let toks = st.tokens_of(tt);
        let (Some(open), Some(close)) = (super::tok_ty_scan::next_sig_idx(toks, 0), super::tok_ty_scan::rfind_sig_idx(toks)) else { return };
        if open >= close {
            return;
        }
        let kind = toks[open].kind;
        if kind == crate::token::TokenKind::LBrace {
            super::tok_ty_scan::for_each_top_comma(&toks[open + 1..close], |clause| {
                if let Some(ni) = super::tok_ty_scan::skip_leading_attrs_vis(clause, 0) {
                    let t = clause[ni];
                    if matches!(t.kind, crate::token::TokenKind::Ident | crate::token::TokenKind::RawIdent) {
                        out.push((crate::sema_fn::norm_ident(t.text(st.src)).to_string(), t.start, t.end));
                    }
                }
            });
        } else if kind == crate::token::TokenKind::LParen {
            let mut idx = 0usize;
            super::tok_ty_scan::for_each_top_comma(&toks[open + 1..close], |clause| {
                if let Some(ti) = super::tok_ty_scan::skip_leading_attrs_vis(clause, 0) {
                    let t = clause[ti];
                    out.push((idx.to_string(), t.start, t.end));
                    idx += 1;
                }
            });
        }
    }
}
