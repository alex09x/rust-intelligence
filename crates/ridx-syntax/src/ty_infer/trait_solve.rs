/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Checkbox `3.4` minimal trait solver, `where`-obligation evaluator, CST `<Param as Trait>::Assoc`
//! projection, and canonical trait `AssocFn` UFCS instantiation (`0` steady-state heap allocations).

use super::autoderef::{normalize_ty_with_owner, NormOwner};
use super::body_ctx::Runner;
use super::db::{TraitAssocFnMeta, TyDefDb, TyLangItem, WhereObligation};
use super::local_defs::LocalCstDefs;
use super::lower_ty::effective_target_def;
pub(super) use super::trait_solve_cst::{check_cst_impl_bounds_in_table, check_cst_impl_bounds_senv, collect_cst_fn_obligations, project_cst_param_assoc};
pub(super) use super::trait_solve_ufcs::{param_has_active_bound, trait_has_super, try_solve_local_cst_impls};
use super::ty::{InferTyId, InferTyKind};
use super::uf_table::InlineTyBuf;
use super::InferenceTable;
use crate::name_res::{FileNameRes, GlobalDefId, ItemKind, ResTarget};
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;
use crate::tree::{NodeId, SyntaxTree};

#[derive(Clone, Copy)]
pub(super) struct SolveEnv<'a, 's> {
    pub local_defs: &'a LocalCstDefs,
    pub st: &'a SyntaxTree<'s>,
    pub file_res: &'a FileNameRes,
    pub db: &'a dyn TyDefDb,
    pub owner: NormOwner,
}

impl<'a, 's> Runner<'a, 's> {
    #[inline]
    pub(super) fn norm_owner(&self) -> NormOwner {
        NormOwner { fn_def: self.fn_def, impl_node: self.root_impl_node, impl_gc: self.impl_gc, fn_node: Some(self.root_fn_node), self_ty: self.self_ty }
    }

    #[inline]
    pub(super) fn norm(&mut self, ty: InferTyId) -> InferTyId {
        let owner = self.norm_owner();
        normalize_ty_with_owner(&mut self.ctx.table, self.db, self.local_defs, self.root_st, self.root_file_res, owner, ty)
    }

    /// Resolves and instantiates a value `PathExpr`, lifting trait `AssocFn`s (`String::from`,
    /// `Default::default`, `T::default`, `<Q as Trait<A>>::f`) to canonical `(trait_fn_def, [Self, TraitArgs.., MethodArgs..])`.
    pub(super) fn infer_value_path(&mut self, path_node: NodeId) -> InferTyId {
        if let Some(sub_ty) = self.lookup_sub_or_expanded_value(path_node) {
            return sub_ty;
        }
        let Some(rref) = self.file_res.path_tail_ref_in_st(self.st, path_node) else {
            let mut segs = self.st.children(path_node).filter(|&c| self.st.kind(c) == K::PathSegment);
            let fseg = segs.next();
            if let Some(fs) = fseg.filter(|_| segs.next().is_none() && !self.st.own_tokens(path_node).any(|(_, t)| t.kind == T::Colon)) {
                if let Some(sty) = self.lookup_sub_local(norm_ident(self.st.text(fs).trim())) {
                    return sty;
                }
            } else if fseg.is_some() {
                let fb = ResTarget::Def { def: GlobalDefId::NONE, local: crate::name_res::LocalItemId::NONE, kind: ItemKind::AssocFn };
                let resolved = self.resolve_unindexed_ufcs_target(path_node, fb);
                if effective_target_def(resolved).is_some() {
                    return self.finish_value_path_target(path_node, resolved);
                }
            }
            return InferTyId::UNKNOWN;
        };
        let target = if matches!(rref.target, ResTarget::Def { def, local, kind: ItemKind::AssocFn } if !def.is_some() && !local.is_some()) {
            self.resolve_unindexed_ufcs_target(path_node, rref.target)
        } else {
            rref.target
        };
        self.finish_value_path_target(path_node, target)
    }

    pub(super) fn finish_value_path_target(&mut self, path_node: NodeId, mut target: ResTarget) -> InferTyId {
        let mut qself_adt: Option<InferTyId> = None;
        if let ResTarget::Def { kind: ItemKind::AssocFn, .. } = target {
            let eff_def = effective_target_def(target);
            if eff_def.is_some() {
                if let Some(meta) = self.db.trait_of_assoc_fn(eff_def).or_else(|| self.local_defs.trait_of_assoc_fn(eff_def, self.root_st, self.root_file_res, self.db)) {
                    return self.instantiate_trait_assoc_fn_path(path_node, meta);
                }
                qself_adt = self.lower_path_qself_adt_ty(path_node);
                if let Some(qself_ty) = qself_adt {
                    let refined = self.db.refine_inherent_assoc_fn(eff_def, qself_ty, &mut self.ctx.table.arena, &mut self.ctx.table.vars);
                    if refined != eff_def && refined.is_some() {
                        target = ResTarget::Def { def: refined, local: crate::name_res::LocalItemId::NONE, kind: ItemKind::AssocFn };
                    }
                }
            }
        } else if matches!(target, ResTarget::Def { kind: ItemKind::Variant, .. }) {
            qself_adt = self.lower_path_qself_adt_ty(path_node);
        }
        let mut explicit = InlineTyBuf::new();
        if matches!(target, ResTarget::Def { kind: ItemKind::Variant, .. }) && qself_adt.is_some() {
            if let Some(InferTyKind::Adt { args, .. }) = qself_adt.map(|t| self.ctx.table.arena.kind(t)) {
                for &a in self.ctx.table.arena.slice(args) {
                    explicit.push(a);
                }
            }
        } else if matches!(target, ResTarget::Def { kind: ItemKind::AssocFn, .. }) && qself_adt.is_some() {
            if let Some(tseg) = self.st.children(path_node).filter(|&c| self.st.kind(c) == K::PathSegment).last() {
                self.ty_env(true).collect_segment_args(&mut self.ctx.table, tseg, false, &mut explicit, 0);
            }
        } else {
            self.collect_path_turbofish(path_node, &mut explicit);
            if explicit.as_slice().is_empty() {
                if let Some(InferTyKind::Adt { args, .. }) = qself_adt.map(|t| self.ctx.table.arena.kind(t)) {
                    for &a in self.ctx.table.arena.slice(args) {
                        explicit.push(a);
                    }
                }
            }
        }
        let val_ty = self.instantiate_target_value(target, explicit.as_slice());
        if let (Some(qself_ty), InferTyKind::FnDef { def, args }) = (qself_adt, self.ctx.table.arena.kind(val_ty)) {
            self.unify_inherent_fn_with_qself(def, args, qself_ty);
        }
        val_ty
    }

    pub(super) fn push_param_bound_cands_with_args(&mut self, st_ty: InferTyId, p_idx: u16, method_name: &str, cands: &mut Vec<super::db::MethodCand>) {
        let mut obls = core::mem::take(&mut self.ctx.where_obls_buf);
        obls.clear();
        if self.fn_def.is_some() {
            self.db.owner_where_obligations(self.fn_def, &mut self.ctx.table.arena, &mut obls);
        }
        let has_local_sub_tr = obls
            .iter()
            .any(|o| matches!(o, WhereObligation::TraitBound { trait_def, trait_args, .. } if trait_args.len == 0 && self.local_defs.traits.iter().any(|t| t.def == *trait_def)));
        if obls.is_empty() || has_local_sub_tr {
            collect_cst_fn_obligations(self.local_defs, self.root_fn_node, self.root_st, self.root_file_res, self.db, &mut self.ctx.table, &mut obls);
        }
        for obl in obls.iter().copied() {
            if let WhereObligation::TraitBound { lhs_ty, trait_def, trait_args } = obl {
                if trait_args.len > 0 && matches!(self.ctx.table.arena.kind(lhs_ty), InferTyKind::Param { idx, .. } if idx == p_idx) {
                    let mut tr_args = InlineTyBuf::new();
                    for &a in self.ctx.table.arena.slice(trait_args) {
                        tr_args.push(a);
                    }
                    if let Some(c) =
                        self.local_defs.trait_decl_method_cand(trait_def, tr_args.as_slice(), st_ty, method_name, self.root_st, self.root_file_res, self.db, &mut self.ctx.table)
                    {
                        cands.push(c);
                    }
                }
            }
        }
        self.ctx.where_obls_buf = obls;
        if p_idx == 0 {
            if let Some(&tr) = self.root_impl_node.filter(|&n| self.root_st.kind(n) == K::Trait).and_then(|im| self.local_defs.traits.iter().find(|t| t.node == im)) {
                let mut tr_args = InlineTyBuf::new();
                for i in 0..tr.trait_generics {
                    tr_args.push(self.ctx.table.arena.mk_param(1 + i, GlobalDefId::NONE));
                }
                if let Some(c) =
                    self.local_defs.trait_decl_method_cand(tr.def, tr_args.as_slice(), st_ty, method_name, self.root_st, self.root_file_res, self.db, &mut self.ctx.table)
                {
                    cands.push(c);
                }
            }
        }
    }

    pub(super) fn instantiate_trait_assoc_fn_path(&mut self, path_node: NodeId, meta: TraitAssocFnMeta) -> InferTyId {
        let mut segs = [NodeId(0); 16];
        let mut slen = 0usize;
        for s in self.st.children(path_node).filter(|&c| self.st.kind(c) == K::PathSegment) {
            if slen < segs.len() {
                segs[slen] = s;
                slen += 1;
            }
        }
        let (first_seg, tail_seg, qual_seg) = ((slen > 0).then(|| segs[0]), (slen > 0).then(|| segs[slen - 1]), (slen >= 2).then(|| segs[slen - 2]));
        let tenv = self.ty_env(true);
        let mut q_targs = InlineTyBuf::new();
        let self_ty = if let Some(fseg) = first_seg.filter(|&fs| self.st.own_tokens(fs).next().is_some_and(|(_, t)| t.kind == T::Lt)) {
            let sty =
                if let Some(tn) = self.st.children(fseg).find(|&c| self.st.kind(c).is_type()) { tenv.lower_cst_ty(&mut self.ctx.table, tn) } else { self.ctx.table.new_ty_var() };
            if let Some(tp_tail) = self.st.child(fseg, K::Path).and_then(|tp| self.st.children(tp).filter(|&c| self.st.kind(c) == K::PathSegment).last()) {
                tenv.collect_segment_args(&mut self.ctx.table, tp_tail, false, &mut q_targs, 0);
            }
            sty
        } else if let Some(qseg) = qual_seg {
            self.lower_qseg_self_ty(path_node, first_seg, qseg, &mut q_targs)
        } else {
            self.ctx.table.new_ty_var()
        };
        let mut m_args = InlineTyBuf::new();
        if let Some(tseg) = tail_seg {
            tenv.collect_segment_args(&mut self.ctx.table, tseg, false, &mut m_args, 0);
        }
        let mut subst = InlineTyBuf::new();
        subst.push(self_ty);
        for i in 0..meta.trait_generics as usize {
            subst.push(q_targs.as_slice().get(i).copied().unwrap_or_else(|| self.ctx.table.new_ty_var()));
        }
        for j in 0..meta.method_generics as usize {
            subst.push(m_args.as_slice().get(j).copied().unwrap_or_else(|| self.ctx.table.new_ty_var()));
        }
        self.ctx.table.arena.mk_fn_def(meta.trait_fn_def, subst.as_slice())
    }

    #[inline]
    pub(super) fn solve_obligation(&mut self, trait_def: GlobalDefId, self_ty: InferTyId, trait_args: &[InferTyId], allow_ambig_self_var: bool, depth: u8) -> bool {
        let senv = SolveEnv { local_defs: self.local_defs, st: self.root_st, file_res: self.root_file_res, db: self.db, owner: self.norm_owner() };
        solve_obligation_senv(senv, trait_def, self_ty, trait_args, &mut self.ctx.table, allow_ambig_self_var, depth)
    }

    pub(super) fn check_local_cst_impl_bounds(&mut self, impl_node: NodeId, impl_gc: u16, subst: &[InferTyId]) -> bool {
        let senv = SolveEnv { local_defs: self.local_defs, st: self.root_st, file_res: self.root_file_res, db: self.db, owner: self.norm_owner() };
        check_cst_impl_bounds_senv(senv, impl_node, impl_gc, subst, &mut self.ctx.table, true, 0)
    }

    /// Evaluates `ProjectionEq` and `TraitBound` obligations after call argument typechecking (`Checkbox 3.4`).
    pub(super) fn solve_call_obligations(&mut self, bound_owner_def: GlobalDefId, cst_fn_node: u32, subst: &[InferTyId]) {
        let mut obls = core::mem::take(&mut self.ctx.where_obls_buf);
        obls.clear();
        if bound_owner_def.is_some() {
            self.db.owner_where_obligations(bound_owner_def, &mut self.ctx.table.arena, &mut obls);
        }
        let indexed_owner = self.local_defs.is_clean_indexed() && bound_owner_def.is_some() && !bound_owner_def.is_overlay();
        if obls.is_empty() && cst_fn_node != u32::MAX && !indexed_owner {
            collect_cst_fn_obligations(self.local_defs, NodeId(cst_fn_node), self.root_st, self.root_file_res, self.db, &mut self.ctx.table, &mut obls);
        }
        for obl in obls.iter().copied() {
            if let WhereObligation::ProjectionEq { lhs_ty, trait_def, assoc_def, trait_args, rhs_ty } = obl {
                let inst_lhs = self.ctx.table.instantiate(lhs_ty, subst);
                let mut inst_args = InlineTyBuf::new();
                for idx in 0..trait_args.len as usize {
                    let a = self.ctx.table.arena.slice(trait_args)[idx];
                    inst_args.push(self.ctx.table.instantiate(a, subst));
                }
                let alias_ty = self.ctx.table.arena.mk_alias(trait_def, assoc_def, inst_lhs, inst_args.as_slice());
                let proj = self.norm(alias_ty);
                let inst_rhs = self.ctx.table.instantiate(rhs_ty, subst);
                if !matches!(self.ctx.table.arena.kind(proj), InferTyKind::Alias { .. } | InferTyKind::Unknown) {
                    let norm_rhs = self.norm(inst_rhs);
                    if matches!(self.ctx.table.arena.kind(norm_rhs), InferTyKind::Alias { .. }) {
                        if self.ctx.deferred_proj_eqs.len() < 8 {
                            self.ctx.deferred_proj_eqs.push((norm_rhs, proj));
                        }
                    } else {
                        let _ = self.ctx.table.coerce(proj, norm_rhs);
                    }
                } else if matches!(self.ctx.table.arena.kind(proj), InferTyKind::Alias { .. }) && self.ctx.deferred_proj_eqs.len() < 8 {
                    self.ctx.deferred_proj_eqs.push((alias_ty, inst_rhs));
                }
            }
        }
        for obl in obls.iter().copied() {
            if let WhereObligation::TraitBound { lhs_ty, trait_def, trait_args } = obl {
                let inst_lhs = self.ctx.table.instantiate(lhs_ty, subst);
                let norm_lhs = self.norm(inst_lhs);
                let mut inst_args = InlineTyBuf::new();
                for idx in 0..trait_args.len as usize {
                    let a = self.ctx.table.arena.slice(trait_args)[idx];
                    let ia = self.ctx.table.instantiate(a, subst);
                    inst_args.push(self.norm(ia));
                }
                if matches!(self.ctx.table.arena.kind(norm_lhs), InferTyKind::Infer(_)) && !inst_args.as_slice().is_empty() && self.ctx.deferred_trait_bounds.len() < 16 {
                    let args_r = self.ctx.table.arena.intern_slice(inst_args.as_slice());
                    self.ctx.deferred_trait_bounds.push((trait_def, norm_lhs, args_r));
                }
                let _ = self.solve_obligation(trait_def, norm_lhs, inst_args.as_slice(), false, 0);
            }
        }
        self.ctx.where_obls_buf = obls;
        if !self.ctx.deferred_proj_eqs.is_empty() {
            self.flush_deferred_proj_eqs();
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn solve_obligation_senv(
    senv: SolveEnv<'_, '_>,
    trait_def: GlobalDefId,
    raw_self: InferTyId,
    raw_args: &[InferTyId],
    table: &mut InferenceTable,
    allow_ambig_self_var: bool,
    depth: u8,
) -> bool {
    if depth > 8 || !trait_def.is_some() {
        return false;
    }
    let self_ty = normalize_ty_with_owner(table, senv.db, senv.local_defs, senv.st, senv.file_res, senv.owner, raw_self);
    let mut targs = InlineTyBuf::new();
    for &a in raw_args {
        targs.push(normalize_ty_with_owner(table, senv.db, senv.local_defs, senv.st, senv.file_res, senv.owner, a));
    }
    match senv.db.lang_item_of_def(trait_def) {
        Some(TyLangItem::Sized) => return !matches!(table.arena.kind(self_ty), InferTyKind::Prim(crate::name_res::PrimTy::Str) | InferTyKind::Slice(_) | InferTyKind::Dyn { .. }),
        Some(TyLangItem::Sync | TyLangItem::Freeze | TyLangItem::Unpin) => return true,
        _ => {}
    }
    match table.arena.kind(self_ty) {
        InferTyKind::Unknown => return allow_ambig_self_var,
        InferTyKind::Infer(vid) => {
            if allow_ambig_self_var {
                return true;
            }
            let is_local_tr = senv.local_defs.traits.iter().any(|t| t.def == trait_def)
                || targs.as_slice().iter().any(|&a| matches!(table.arena.kind(a), InferTyKind::Adt { def, .. } if senv.local_defs.adt_generic_count(def).is_some()));
            if (is_local_tr || !matches!(table.vars.probe_var(vid), Err((_, super::ty::TyVarKind::General, ..))))
                && try_solve_local_cst_impls(senv, trait_def, self_ty, targs.as_slice(), table, depth + 1)
            {
                return true;
            }
            let _ = senv.db.solve_trait_obligation(trait_def, self_ty, targs.as_slice(), &mut table.arena, &mut table.vars);
            return true;
        }
        _ => {}
    }
    if matches!(senv.db.lang_item_of_def(trait_def), Some(TyLangItem::Fn | TyLangItem::FnMut | TyLangItem::FnOnce)) {
        let mut fn_self = self_ty;
        while let InferTyKind::Ref { inner, .. } = table.arena.kind(fn_self) {
            fn_self = table.shallow_resolve(inner);
        }
        if let InferTyKind::Closure { sig, .. } | InferTyKind::FnPtr { sig } = table.arena.kind(fn_self) {
            if sig.len >= 1 {
                let mut sb = InlineTyBuf::new();
                for &x in table.arena.slice(sig) {
                    sb.push(x);
                }
                let sl = sb.as_slice();
                let (ret_ty, tup) = (sl[sl.len() - 1], table.arena.mk_tuple(&sl[..sl.len() - 1]));
                let snap = table.start_snapshot();
                if targs.as_slice().first().is_none_or(|&a0| table.equate(a0, tup).is_ok()) && targs.as_slice().get(1).is_none_or(|&a1| table.coerce(ret_ty, a1).is_ok()) {
                    table.commit(snap);
                    return true;
                }
                table.rollback_to(snap);
                return false;
            }
        }
    }
    if let InferTyKind::Param { idx, def: pdef } = table.arena.kind(self_ty) {
        if !targs.as_slice().is_empty() {
            let _ = senv.db.solve_trait_obligation(trait_def, self_ty, targs.as_slice(), &mut table.arena, &mut table.vars);
        }
        if param_has_active_bound(senv, table, idx, pdef, trait_def) {
            return true;
        }
    }
    if let InferTyKind::Dyn { principal, args } | InferTyKind::Opaque { def: principal, args } = table.arena.kind(self_ty) {
        if principal == trait_def {
            let mut oa = InlineTyBuf::new();
            for &x in table.arena.slice(args) {
                oa.push(x);
            }
            return targs.as_slice().iter().zip(oa.as_slice()).all(|(&ta, &o)| table.equate(ta, o).is_ok());
        }
        if trait_has_super(senv, table, principal, trait_def) {
            return true;
        }
    }
    try_solve_local_cst_impls(senv, trait_def, self_ty, targs.as_slice(), table, depth + 1)
        || senv.db.solve_trait_obligation(trait_def, self_ty, targs.as_slice(), &mut table.arena, &mut table.vars)
}
