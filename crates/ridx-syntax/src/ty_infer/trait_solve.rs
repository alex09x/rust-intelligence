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

use super::autoderef::{impl_target_key_of_ty, normalize_ty_with_owner, NormOwner};
use super::body_ctx::Runner;
use super::db::{TraitAssocFnMeta, TyDefDb, TyLangItem, WhereObligation};
use super::local_defs::LocalCstDefs;
use super::lower_ty::effective_target_def;
pub(super) use super::trait_solve_cst::{
    check_cst_impl_bounds_in_table, check_cst_impl_bounds_senv, collect_cst_fn_obligations, param_has_active_bound, project_cst_param_assoc, trait_has_super,
};
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
            return InferTyId::UNKNOWN;
        };
        let target = if matches!(rref.target, ResTarget::Def { def, local, kind: ItemKind::AssocFn } if !def.is_some() && !local.is_some()) {
            self.resolve_unindexed_ufcs_target(path_node, rref.target)
        } else {
            rref.target
        };
        if let ResTarget::Def { kind: ItemKind::AssocFn, .. } = target {
            let eff_def = effective_target_def(target);
            if eff_def.is_some() {
                let meta_opt = self.db.trait_of_assoc_fn(eff_def).or_else(|| self.local_defs.trait_of_assoc_fn(eff_def, self.root_st, self.root_file_res, self.db));
                if let Some(meta) = meta_opt {
                    return self.instantiate_trait_assoc_fn_path(path_node, meta);
                }
            }
        }
        let mut explicit = InlineTyBuf::new();
        self.collect_path_turbofish(path_node, &mut explicit);
        self.instantiate_target_value(target, explicit.as_slice())
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
        let first_seg = (slen > 0).then(|| segs[0]);
        let tail_seg = (slen > 0).then(|| segs[slen - 1]);
        let qual_seg = (slen >= 2).then(|| segs[slen - 2]);

        let tenv = self.ty_env(true);
        let mut q_targs = InlineTyBuf::new();
        let self_ty = if let Some(fseg) = first_seg.filter(|&fs| self.st.own_tokens(fs).next().is_some_and(|(_, t)| t.kind == T::Lt)) {
            let sty = match self.st.children(fseg).find(|&c| self.st.kind(c).is_type()) {
                Some(tn) => tenv.lower_cst_ty(&mut self.ctx.table, tn),
                None => self.ctx.table.new_ty_var(),
            };
            if let Some(tp) = self.st.child(fseg, K::Path) {
                if let Some(tp_tail) = self.st.children(tp).filter(|&c| self.st.kind(c) == K::PathSegment).last() {
                    tenv.collect_segment_args(&mut self.ctx.table, tp_tail, false, &mut q_targs, 0);
                }
            }
            sty
        } else if let Some(qseg) = qual_seg {
            let qr = self.st.range(qseg);
            let q_target = self
                .file_res
                .refs_starting_in(qr.start as u32, qr.end as u32)
                .iter()
                .find(|r| (r.path_node == path_node || r.path_node == NodeId(u32::MAX)) && r.end <= qr.end as u32)
                .map(|r| r.target);
            match q_target {
                Some(ResTarget::Def { kind: ItemKind::Trait, .. }) => {
                    tenv.collect_segment_args(&mut self.ctx.table, qseg, false, &mut q_targs, 0);
                    self.ctx.table.new_ty_var()
                }
                Some(qt) => {
                    let mut fargs = InlineTyBuf::new();
                    tenv.collect_segment_args(&mut self.ctx.table, qseg, false, &mut fargs, 0);
                    tenv.finish_ufcs_qself_type(&mut self.ctx.table, qt, qseg, fargs.as_slice())
                }
                None => {
                    if self.st.own_tokens(qseg).any(|(_, t)| t.kind == T::SelfType) {
                        self.self_ty.unwrap_or_else(|| self.ctx.table.new_ty_var())
                    } else {
                        crate::name_res::PrimTy::from_name(norm_ident(self.st.text(qseg))).map_or_else(|| self.ctx.table.new_ty_var(), InferTyId::from_prim)
                    }
                }
            }
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
                if !matches!(self.ctx.table.arena.kind(proj), InferTyKind::Alias { .. } | InferTyKind::Unknown) {
                    let inst_rhs = self.ctx.table.instantiate(rhs_ty, subst);
                    let norm_rhs = self.norm(inst_rhs);
                    let _ = self.ctx.table.coerce(proj, norm_rhs);
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
                let _ = self.solve_obligation(trait_def, norm_lhs, inst_args.as_slice(), false, 0);
            }
        }
        self.ctx.where_obls_buf = obls;
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
    match table.arena.kind(self_ty) {
        InferTyKind::Unknown => return allow_ambig_self_var,
        InferTyKind::Infer(_) => {
            if allow_ambig_self_var {
                return true;
            }
            let _ = try_solve_local_cst_impls(senv, trait_def, self_ty, targs.as_slice(), table, depth + 1)
                || senv.db.solve_trait_obligation(trait_def, self_ty, targs.as_slice(), &mut table.arena, &mut table.vars);
            return true;
        }
        _ => {}
    }
    if matches!(senv.db.lang_item_of_def(trait_def), Some(TyLangItem::Fn | TyLangItem::FnMut | TyLangItem::FnOnce)) {
        if let InferTyKind::Closure { sig, .. } | InferTyKind::FnPtr { sig } = table.arena.kind(self_ty) {
            if sig.len >= 1 {
                let mut sb = InlineTyBuf::new();
                for &x in table.arena.slice(sig) {
                    sb.push(x);
                }
                let sl = sb.as_slice();
                let ret_ty = sl[sl.len() - 1];
                let tup = table.arena.mk_tuple(&sl[..sl.len() - 1]);
                let snap = table.start_snapshot();
                let ok1 = targs.as_slice().first().is_none_or(|&a0| table.equate(a0, tup).is_ok());
                let ok2 = targs.as_slice().get(1).is_none_or(|&a1| table.coerce(ret_ty, a1).is_ok());
                if ok1 && ok2 {
                    table.commit(snap);
                    return true;
                }
                table.rollback_to(snap);
                return false;
            }
        }
    }
    if let InferTyKind::Param { idx, def: pdef } = table.arena.kind(self_ty) {
        if param_has_active_bound(senv, table, idx, pdef, trait_def) {
            return true;
        }
    }
    if let InferTyKind::Dyn { principal, .. } | InferTyKind::Opaque { def: principal, .. } = table.arena.kind(self_ty) {
        if principal == trait_def || trait_has_super(senv, table, principal, trait_def) {
            return true;
        }
    }
    if try_solve_local_cst_impls(senv, trait_def, self_ty, targs.as_slice(), table, depth + 1) {
        return true;
    }
    senv.db.solve_trait_obligation(trait_def, self_ty, targs.as_slice(), &mut table.arena, &mut table.vars)
}

fn try_solve_local_cst_impls(senv: SolveEnv<'_, '_>, trait_def: GlobalDefId, self_ty: InferTyId, trait_args: &[InferTyId], table: &mut InferenceTable, depth: u8) -> bool {
    let self_head = impl_target_key_of_ty(&table.arena, self_ty);
    let self_is_var = matches!(table.arena.kind(self_ty), InferTyKind::Infer(_));
    let mut match_count = 0u32;
    let mut unique_imp: Option<(NodeId, u16)> = None;
    for imp in &senv.local_defs.impls {
        if imp.indexed_in_clean_db || !imp.has_for || imp.is_negative {
            continue;
        }
        let Some((imp_tr, imp_args, imp_self)) = senv.local_defs.lower_trait_impl_header(imp.node, imp.impl_generics, senv.st, senv.file_res, senv.db, table) else {
            continue;
        };
        if imp_tr != trait_def {
            continue;
        }
        let is_blanket = matches!(table.arena.kind(imp_self), InferTyKind::Param { .. });
        if !self_is_var && !is_blanket && impl_target_key_of_ty(&table.arena, imp_self) != self_head {
            continue;
        }
        let snap = table.start_snapshot();
        let ok = bind_and_check_cst_impl(senv, (imp.node, imp.impl_generics, imp_self, is_blanket), imp_args.as_slice(), self_ty, trait_args, table, depth);
        table.rollback_to(snap);
        if ok {
            match_count += 1;
            unique_imp = Some((imp.node, imp.impl_generics));
            if match_count > 1 {
                break;
            }
        }
    }
    if match_count == 1 {
        if let Some((node, igc)) = unique_imp {
            if let Some((_, imp_args, imp_self)) = senv.local_defs.lower_trait_impl_header(node, igc, senv.st, senv.file_res, senv.db, table) {
                let is_b = matches!(table.arena.kind(imp_self), InferTyKind::Param { .. });
                let snap = table.start_snapshot();
                if bind_and_check_cst_impl(senv, (node, igc, imp_self, is_b), imp_args.as_slice(), self_ty, trait_args, table, depth) {
                    table.commit(snap);
                } else {
                    table.rollback_to(snap);
                }
            }
        }
    }
    match_count >= 1
}

#[allow(clippy::too_many_arguments)]
fn bind_and_check_cst_impl(
    senv: SolveEnv<'_, '_>,
    (imp_node, imp_gc, imp_self, is_blanket): (NodeId, u16, InferTyId, bool),
    imp_args: &[InferTyId],
    self_ty: InferTyId,
    trait_args: &[InferTyId],
    table: &mut InferenceTable,
    depth: u8,
) -> bool {
    let mut subst = InlineTyBuf::new();
    for _ in 0..imp_gc {
        subst.push(table.new_ty_var());
    }
    let inst_self = table.instantiate(imp_self, subst.as_slice());
    if table.equate(self_ty, inst_self).is_err() {
        return false;
    }
    if !trait_args.is_empty() {
        if imp_args.len() == trait_args.len() {
            for (i, &ta) in trait_args.iter().enumerate() {
                let ia = table.instantiate(imp_args[i], subst.as_slice());
                if table.equate(ta, ia).is_err() {
                    return false;
                }
            }
        } else if !imp_args.is_empty() || trait_args.len() != 1 || table.equate(trait_args[0], inst_self).is_err() {
            return false;
        }
    }
    !is_blanket || check_cst_impl_bounds_senv(senv, imp_node, imp_gc, subst.as_slice(), table, false, depth)
}
