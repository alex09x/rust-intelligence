/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Checkbox `3.4` `.ridx` `v5` trait obligation solver, `impl` selection with generic arguments,
//! `WherePred` bound checker, `owner_where_obligations`, and `<Param as Trait>::Assoc` projection.

use ridx_format::{TyKindView, WherePredKind};
use ridx_syntax::name_res::{GlobalDefId, ImplTargetKey, PrimTy};
use ridx_syntax::ty_infer::autoderef::impl_target_key_of_ty;
use ridx_syntax::ty_infer::uf_table::InlineTyBuf;
use ridx_syntax::ty_infer::{unify, InferTyArena, InferTyId, InferTyKind, MethodCand, TraitAssocFnMeta, TyLangItem, TyVarKind, UnionFindTable, WhereObligation};

use crate::v5_db::RidxV5ItemDb;
use crate::v5_ty_lower::{lower_generic_args, lower_v5_ty, ParamRemap};
use crate::v5_ty_proj::instantiate_v5;
use crate::v5_ty_tables::{fnv1a_str, TraitImplInfo};

impl<'w> RidxV5ItemDb<'w> {
    #[inline]
    pub fn v5_trait_of_assoc_fn(&self, raw_def: GlobalDefId) -> Option<TraitAssocFnMeta> {
        self.ty_tables.trait_fn_meta.get(&self.canonical_def(raw_def)).copied()
    }

    pub fn v5_trait_assoc_fn_by_name(&self, raw_trait: GlobalDefId, fn_name: &str) -> Option<TraitAssocFnMeta> {
        let tgid = self.canonical_def(raw_trait);
        let nh = fnv1a_str(fn_name);
        if let Some(&tm) = self.ty_tables.trait_assoc_fns.get(&(tgid, nh)).filter(|i| i.name == fn_name) {
            return Some(TraitAssocFnMeta {
                trait_def: tgid,
                trait_fn_def: GlobalDefId::new(tgid.krate, tm.method_didx),
                trait_generics: tm.trait_gc,
                method_generics: tm.method_gc,
            });
        }
        if let Some(supers) = self.trait_supertraits.get(&tgid) {
            for &sup in supers {
                let csup = self.canonical_def(sup);
                if let Some(&tm) = self.ty_tables.trait_assoc_fns.get(&(csup, nh)).filter(|i| i.name == fn_name) {
                    return Some(TraitAssocFnMeta {
                        trait_def: csup,
                        trait_fn_def: GlobalDefId::new(csup.krate, tm.method_didx),
                        trait_generics: tm.trait_gc,
                        method_generics: tm.method_gc,
                    });
                }
            }
        }
        None
    }

    pub fn v5_owner_where_obligations(&self, raw_owner: GlobalDefId, arena: &mut InferTyArena, out: &mut Vec<WhereObligation>) {
        let owner = self.canonical_def(raw_owner);
        let r_idx = owner.krate.0 as usize;
        let Some(reader) = self.ws.readers().get(r_idx) else { return };
        let remap = ParamRemap::for_def(self, r_idx, owner.index, false, arena);
        let va = reader.ty_arena();
        for wp in self.iter_owner_where_preds(r_idx, owner.index) {
            match wp.kind() {
                WherePredKind::TraitBound if !wp.is_negative() => {
                    let Some(td) = wp.trait_def() else { continue };
                    let tgid = self.ty_def_gid(r_idx, td);
                    if !tgid.is_some() {
                        continue;
                    }
                    if matches!(self.ty_tables.gid_to_lang.get(&tgid).copied(), Some(TyLangItem::Sized | TyLangItem::Sync | TyLangItem::Freeze | TyLangItem::Unpin)) {
                        continue;
                    }
                    let lhs_ty = lower_v5_ty(self, r_idx, va, wp.lhs_ty(), &remap, arena);
                    let mut targs = InlineTyBuf::new();
                    lower_generic_args(self, r_idx, va, wp.trait_args(), &remap, arena, &mut targs, 1);
                    let trait_args = arena.intern_slice(targs.as_slice());
                    out.push(WhereObligation::TraitBound { lhs_ty, trait_def: tgid, trait_args });
                }
                WherePredKind::ProjectionEq => {
                    let (Some(td), Some(aname), Some(rhs)) = (wp.trait_def(), wp.assoc_name(), wp.rhs_ty()) else { continue };
                    let tgid = self.ty_def_gid(r_idx, td);
                    let assoc_def = self.lookup_trait_assoc_ty_def(tgid, aname);
                    if !tgid.is_some() || !assoc_def.is_some() {
                        continue;
                    }
                    let lhs_ty = lower_v5_ty(self, r_idx, va, wp.lhs_ty(), &remap, arena);
                    let mut targs = InlineTyBuf::new();
                    lower_generic_args(self, r_idx, va, wp.trait_args(), &remap, arena, &mut targs, 1);
                    let trait_args = arena.intern_slice(targs.as_slice());
                    let rhs_ty = lower_v5_ty(self, r_idx, va, rhs, &remap, arena);
                    out.push(WhereObligation::ProjectionEq { lhs_ty, trait_def: tgid, assoc_def, trait_args, rhs_ty });
                }
                _ => {}
            }
        }
    }

    pub fn v5_check_blanket_method_bounds(&self, cand: &MethodCand, subst: &[InferTyId], arena: &mut InferTyArena, vars: &mut UnionFindTable) -> bool {
        let (r_idx, impl_idx) = if cand.impl_owner.is_some() && !cand.impl_owner.is_overlay() {
            (cand.impl_owner.krate.0 as usize, cand.impl_owner.index)
        } else {
            let mdef = self.canonical_def(cand.method_def);
            let Some(&idx) = self.ty_tables.assoc_to_impl.get(&mdef) else { return true };
            (mdef.krate.0 as usize, idx)
        };
        if r_idx >= self.ws.crate_count() {
            return true;
        }
        let remap = ParamRemap::for_impl(self, r_idx, impl_idx, arena);
        self.check_impl_where_bounds_ext(r_idx, impl_idx, &remap, subst, arena, vars, true, 0)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn check_impl_where_bounds_ext(
        &self,
        r_idx: usize,
        impl_idx: u32,
        remap: &ParamRemap,
        subst: &[InferTyId],
        arena: &mut InferTyArena,
        vars: &mut UnionFindTable,
        allow_ambig_self_var: bool,
        depth: u8,
    ) -> bool {
        if depth > 2 {
            return true;
        }
        let reader = &self.ws.readers()[r_idx];
        let va = reader.ty_arena();
        for wp in reader.where_preds_for_impl(impl_idx) {
            if wp.kind() != WherePredKind::TraitBound || wp.is_negative() {
                continue;
            }
            let Some(td) = wp.trait_def() else { continue };
            let b_trait = self.ty_def_gid(r_idx, td);
            if !b_trait.is_some() {
                continue;
            }
            let raw_lhs = lower_v5_ty(self, r_idx, va, wp.lhs_ty(), remap, arena);
            let tmp_lhs = instantiate_v5(arena, raw_lhs, subst, 0);
            let inst_lhs = vars.shallow_resolve(arena, tmp_lhs);
            let mut raw_targs = InlineTyBuf::new();
            lower_generic_args(self, r_idx, va, wp.trait_args(), remap, arena, &mut raw_targs, 1);
            let mut inst_targs = InlineTyBuf::new();
            for &ta in raw_targs.as_slice() {
                inst_targs.push(instantiate_v5(arena, ta, subst, 0));
            }
            if !self.v5_solve_obligation_rec(b_trait, inst_lhs, inst_targs.as_slice(), arena, vars, allow_ambig_self_var, depth + 1) {
                return false;
            }
        }
        true
    }

    pub fn v5_solve_trait_obligation(&self, raw_trait: GlobalDefId, raw_self: InferTyId, trait_args: &[InferTyId], arena: &mut InferTyArena, vars: &mut UnionFindTable) -> bool {
        self.v5_solve_obligation_rec(raw_trait, raw_self, trait_args, arena, vars, false, 0)
    }

    #[allow(clippy::too_many_arguments)]
    fn v5_solve_obligation_rec(
        &self,
        raw_trait: GlobalDefId,
        raw_self: InferTyId,
        trait_args: &[InferTyId],
        arena: &mut InferTyArena,
        vars: &mut UnionFindTable,
        allow_ambig_self_var: bool,
        depth: u8,
    ) -> bool {
        if depth > 3 {
            return true;
        }
        let trait_gid = self.canonical_def(raw_trait);
        let cur = vars.shallow_resolve(arena, raw_self);
        match arena.kind(cur) {
            InferTyKind::Infer(_) => return allow_ambig_self_var,
            InferTyKind::Unknown | InferTyKind::Never | InferTyKind::Alias { .. } | InferTyKind::Param { .. } => return true,
            InferTyKind::Dyn { principal, .. } | InferTyKind::Opaque { def: principal, .. } => {
                let cp = self.canonical_def(principal);
                if cp == trait_gid || self.trait_supertraits.get(&cp).is_some_and(|s| s.iter().any(|&x| self.canonical_def(x) == trait_gid)) {
                    return true;
                }
            }
            _ => {}
        }
        match self.ty_tables.gid_to_lang.get(&trait_gid).copied() {
            Some(TyLangItem::Sized) => return !matches!(arena.kind(cur), InferTyKind::Prim(PrimTy::Str) | InferTyKind::Slice(_) | InferTyKind::Dyn { .. }),
            Some(TyLangItem::Sync | TyLangItem::Freeze | TyLangItem::Unpin) => return true,
            Some(TyLangItem::Copy | TyLangItem::Clone) => {
                if matches!(arena.kind(cur), InferTyKind::Prim(p) if p != PrimTy::Str)
                    || matches!(arena.kind(cur), InferTyKind::Ref { is_mut: false, .. } | InferTyKind::Ptr { .. } | InferTyKind::FnPtr { .. } | InferTyKind::FnDef { .. })
                {
                    return true;
                }
            }
            Some(l) if l.is_fn_trait() => {
                if matches!(arena.kind(cur), InferTyKind::Closure { .. } | InferTyKind::FnPtr { .. } | InferTyKind::FnDef { .. }) {
                    return true;
                }
            }
            _ => {}
        }

        let raw_head = impl_target_key_of_ty(arena, cur);
        let head = match raw_head {
            ImplTargetKey::Adt(d) => ImplTargetKey::Adt(self.canonical_def(d)),
            ImplTargetKey::Dyn(d) => ImplTargetKey::Dyn(self.canonical_def(d)),
            other => other,
        };
        let concrete = self.ty_tables.concrete_trait_impls.get(&(trait_gid, head)).map_or(&[][..], |v| v.as_slice());
        let blanket = self.ty_tables.blanket_trait_impls.get(&trait_gid).map_or(&[][..], |v| v.as_slice());

        let mut match_count = 0usize;
        let mut unique_imp: Option<TraitImplInfo> = None;
        for &imp in concrete.iter().chain(blanket.iter()) {
            let snap = vars.start_snapshot();
            let ok = self.try_bind_v5_trait_impl(&imp, cur, trait_args, arena, vars, depth);
            vars.rollback_to(snap);
            if ok {
                match_count += 1;
                if match_count == 1 {
                    unique_imp = Some(imp);
                } else {
                    break;
                }
            }
        }
        if match_count == 1 {
            if let Some(imp) = unique_imp {
                let snap = vars.start_snapshot();
                if self.try_bind_v5_trait_impl(&imp, cur, trait_args, arena, vars, depth) {
                    vars.commit(snap);
                } else {
                    vars.rollback_to(snap);
                }
            }
        }
        match_count >= 1
    }

    fn try_bind_v5_trait_impl(&self, imp: &TraitImplInfo, cur: InferTyId, trait_args: &[InferTyId], arena: &mut InferTyArena, vars: &mut UnionFindTable, depth: u8) -> bool {
        let r_idx = imp.r_idx as usize;
        let reader = &self.ws.readers()[r_idx];
        let Some(iv) = reader.impl_view(imp.impl_idx) else { return false };
        let va = reader.ty_arena();
        let remap = ParamRemap::for_impl(self, r_idx, imp.impl_idx, arena);
        let raw_self = lower_v5_ty(self, r_idx, va, iv.self_ty(), &remap, arena);
        let mut raw_targs = InlineTyBuf::new();
        lower_generic_args(self, r_idx, va, iv.trait_args(), &remap, arena, &mut raw_targs, 1);

        let mut subst = InlineTyBuf::new();
        for _ in 0..imp.impl_gc {
            subst.push(vars.new_var(arena, TyVarKind::General).1);
        }
        let inst_self = instantiate_v5(arena, raw_self, subst.as_slice(), 0);
        if unify::equate(arena, vars, cur, inst_self).is_err() {
            return false;
        }
        if !trait_args.is_empty() {
            if raw_targs.as_slice().len() < trait_args.len() {
                return false;
            }
            for (i, &ta) in trait_args.iter().enumerate() {
                let ia = instantiate_v5(arena, raw_targs.as_slice()[i], subst.as_slice(), 0);
                if unify::equate(arena, vars, ta, ia).is_err() {
                    return false;
                }
            }
        }
        (imp.impl_gc == 0 && !imp.is_blanket) || self.check_impl_where_bounds_ext(r_idx, imp.impl_idx, &remap, subst.as_slice(), arena, vars, false, depth + 1)
    }

    pub fn v5_project_param_assoc(
        &self,
        raw_owner: GlobalDefId,
        param_idx: u16,
        raw_trait: GlobalDefId,
        raw_assoc: GlobalDefId,
        trait_args: &[InferTyId],
        arena: &mut InferTyArena,
    ) -> Option<InferTyId> {
        let owner = self.canonical_def(raw_owner);
        let tgid = self.canonical_def(raw_trait);
        let agid = self.canonical_def(raw_assoc);
        let aname = *self.ty_tables.assoc_ty_names.get(&agid)?;
        let r_idx = owner.krate.0 as usize;
        let reader = self.ws.readers().get(r_idx)?;
        let remap = ParamRemap::for_def(self, r_idx, owner.index, false, arena);
        let va = reader.ty_arena();

        for wp in self.iter_owner_where_preds(r_idx, owner.index) {
            if wp.kind() != WherePredKind::ProjectionEq || wp.assoc_name() != Some(aname) {
                continue;
            }
            let TyKindView::Param { index, .. } = va.kind(wp.lhs_ty()) else { continue };
            if remap.dense_idx_of(index, arena) != Some(param_idx) {
                continue;
            }
            if tgid.is_some() {
                let Some(td) = wp.trait_def() else { continue };
                let b_tr = self.ty_def_gid(r_idx, td);
                let tr_ok = b_tr == tgid || self.trait_supertraits.get(&b_tr).is_some_and(|s| s.iter().any(|&x| self.canonical_def(x) == tgid));
                if !tr_ok {
                    continue;
                }
            }
            if !trait_args.is_empty() {
                let mut wp_targs = InlineTyBuf::new();
                lower_generic_args(self, r_idx, va, wp.trait_args(), &remap, arena, &mut wp_targs, 1);
                if wp_targs.as_slice() != trait_args {
                    continue;
                }
            }
            if let Some(rhs) = wp.rhs_ty() {
                return Some(lower_v5_ty(self, r_idx, va, rhs, &remap, arena));
            }
        }
        None
    }
}
