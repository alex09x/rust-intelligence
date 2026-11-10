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

use ridx_format::WherePredKind;
use ridx_syntax::name_res::{GlobalDefId, ImplTargetKey, PrimTy};
use ridx_syntax::ty_infer::autoderef::impl_target_key_of_ty;
use ridx_syntax::ty_infer::uf_table::InlineTyBuf;
use ridx_syntax::ty_infer::{
    unify, vars_has_unresolved_infer, InferTyArena, InferTyId, InferTyKind, MethodCand, TraitAssocFnMeta, TyLangItem, TyVarKind, UnionFindTable, WhereObligation,
};

use super::v5_db::RidxV5ItemDb;
use super::v5_ty_lower::{lower_generic_args, lower_v5_ty, ParamRemap};
use super::v5_ty_proj::instantiate_v5;
use super::v5_ty_tables::{fnv1a_str, TraitImplInfo};

impl<'w> RidxV5ItemDb<'w> {
    #[inline]
    pub fn v5_trait_of_assoc_fn(&self, raw_def: GlobalDefId) -> Option<TraitAssocFnMeta> {
        self.ty_tables.trait_fn_meta.get(&self.canonical_def(raw_def)).copied()
    }

    pub fn v5_trait_assoc_fn_by_name(&self, raw_trait: GlobalDefId, fn_name: &str) -> Option<TraitAssocFnMeta> {
        let tgid = self.canonical_def(raw_trait);
        let nh = fnv1a_str(fn_name);
        if let Some(&tm) = self.ty_tables.trait_assoc_fns.get(&(tgid, nh)).filter(|i| i.name.as_str(self.ws) == fn_name) {
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
                if let Some(&tm) = self.ty_tables.trait_assoc_fns.get(&(csup, nh)).filter(|i| i.name.as_str(self.ws) == fn_name) {
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
                    if !tgid.is_some()
                        || matches!(self.ty_tables.gid_to_lang.get(&tgid).copied(), Some(TyLangItem::Sized | TyLangItem::Sync | TyLangItem::Freeze | TyLangItem::Unpin))
                    {
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
        if depth > if allow_ambig_self_var { 3 } else { 4 } {
            return true;
        }
        let reader = &self.ws.readers()[r_idx];
        let va = reader.ty_arena();
        let (mut has_proj_eq, mut deferred_mask) = (false, 0u64);
        for (pred_idx, wp) in reader.where_preds_for_impl(impl_idx).enumerate() {
            if wp.kind() == WherePredKind::ProjectionEq {
                has_proj_eq = true;
                continue;
            }
            if wp.kind() != WherePredKind::TraitBound || wp.is_negative() {
                continue;
            }
            let Some(td) = wp.trait_def() else { continue };
            let b_trait = self.ty_def_gid(r_idx, td);
            if !b_trait.is_some() {
                continue;
            }
            let raw_lhs = lower_v5_ty(self, r_idx, va, wp.lhs_ty(), remap, arena);
            let ilhs = instantiate_v5(arena, raw_lhs, subst, 0);
            let tmp_lhs = vars.shallow_resolve(arena, ilhs);
            let inst_lhs = if matches!(arena.kind(tmp_lhs), InferTyKind::Alias { .. }) { self.norm_v5_top_alias(tmp_lhs, arena, vars, depth + 1) } else { tmp_lhs };
            if matches!(arena.kind(inst_lhs), InferTyKind::Infer(_)) || (matches!(arena.kind(inst_lhs), InferTyKind::Alias { .. }) && arena.flags(inst_lhs).has_infer()) {
                deferred_mask |= 1u64 << pred_idx.min(63);
                continue;
            }
            let mut raw_targs = InlineTyBuf::new();
            lower_generic_args(self, r_idx, va, wp.trait_args(), remap, arena, &mut raw_targs, 1);
            if self.ty_tables.gid_to_lang.get(&self.canonical_def(b_trait)).is_some_and(|l| l.is_fn_trait()) && raw_targs.as_slice().iter().any(|&t| arena.flags(t).has_alias()) {
                continue;
            }
            let mut inst_targs = InlineTyBuf::new();
            for &ta in raw_targs.as_slice() {
                inst_targs.push(instantiate_v5(arena, ta, subst, 0));
            }
            if !self.v5_solve_obligation_rec(b_trait, inst_lhs, inst_targs.as_slice(), arena, vars, allow_ambig_self_var, depth + 1) {
                return false;
            }
        }
        if has_proj_eq {
            self.eval_impl_proj_eqs(r_idx, impl_idx, remap, subst, arena, vars, depth + 1);
        }
        if has_proj_eq && deferred_mask != 0 {
            for (pred_idx, wp) in reader.where_preds_for_impl(impl_idx).enumerate() {
                if (deferred_mask & (1u64 << pred_idx.min(63))) == 0 || wp.kind() != WherePredKind::TraitBound || wp.is_negative() {
                    continue;
                }
                let Some(td) = wp.trait_def() else { continue };
                let b_trait = self.ty_def_gid(r_idx, td);
                if !b_trait.is_some() {
                    continue;
                }
                let raw_lhs = lower_v5_ty(self, r_idx, va, wp.lhs_ty(), remap, arena);
                let ilhs = instantiate_v5(arena, raw_lhs, subst, 0);
                let tmp_lhs = vars.shallow_resolve(arena, ilhs);
                let inst_lhs = if matches!(arena.kind(tmp_lhs), InferTyKind::Alias { .. }) { self.norm_v5_top_alias(tmp_lhs, arena, vars, depth + 1) } else { tmp_lhs };
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
        if depth > if allow_ambig_self_var { 4 } else { 5 } {
            return true;
        }
        let trait_gid = self.canonical_def(raw_trait);
        let cur = vars.shallow_resolve(arena, raw_self);
        match self.ty_tables.gid_to_lang.get(&trait_gid).copied() {
            Some(TyLangItem::Sized) => return !matches!(arena.kind(cur), InferTyKind::Prim(PrimTy::Str) | InferTyKind::Slice(_) | InferTyKind::Dyn { .. }),
            Some(TyLangItem::Sync | TyLangItem::Freeze | TyLangItem::Unpin) => return true,
            _ => {}
        }
        match arena.kind(cur) {
            InferTyKind::Infer(v) => {
                if !allow_ambig_self_var {
                    match vars.probe_var(v) {
                        Err((_, TyVarKind::IntVar, ..)) => {
                            const INT_PRIMS: [PrimTy; 12] = [
                                PrimTy::Usize,
                                PrimTy::Isize,
                                PrimTy::U8,
                                PrimTy::U16,
                                PrimTy::U32,
                                PrimTy::U64,
                                PrimTy::U128,
                                PrimTy::I8,
                                PrimTy::I16,
                                PrimTy::I32,
                                PrimTy::I64,
                                PrimTy::I128,
                            ];
                            self.try_bind_unique_prim_trait_impl(trait_gid, cur, trait_args, &INT_PRIMS, arena, vars, depth);
                        }
                        Err((_, TyVarKind::FloatVar, ..)) => {
                            const FLT_PRIMS: [PrimTy; 2] = [PrimTy::F32, PrimTy::F64];
                            self.try_bind_unique_prim_trait_impl(trait_gid, cur, trait_args, &FLT_PRIMS, arena, vars, depth);
                        }
                        _ => {}
                    }
                }
                return allow_ambig_self_var;
            }
            InferTyKind::Param { idx, .. } => {
                if !trait_args.is_empty() {
                    self.try_bind_active_param_trait_args(idx, trait_gid, trait_args, arena, vars);
                }
                return true;
            }
            InferTyKind::Unknown | InferTyKind::Never | InferTyKind::Alias { .. } => return true,
            InferTyKind::Dyn { principal, args } | InferTyKind::Opaque { def: principal, args } => {
                let cp = self.canonical_def(principal);
                let mut oa = InlineTyBuf::new();
                for &x in arena.slice(args) {
                    oa.push(x);
                }
                if cp == trait_gid {
                    return trait_args.iter().zip(oa.as_slice()).all(|(&ta, &o)| unify::equate(arena, vars, ta, o).is_ok());
                }
                if self.trait_supertraits.get(&cp).is_some_and(|s| s.iter().any(|&x| self.canonical_def(x) == trait_gid)) {
                    if !trait_args.is_empty() {
                        self.equate_supertrait_args(cp, trait_gid, cur, oa.as_slice(), trait_args, arena, vars);
                    }
                    return true;
                }
            }
            _ => {}
        }
        match self.ty_tables.gid_to_lang.get(&trait_gid).copied() {
            Some(TyLangItem::Copy | TyLangItem::Clone) => {
                if matches!(arena.kind(cur), InferTyKind::Prim(p) if p != PrimTy::Str)
                    || matches!(arena.kind(cur), InferTyKind::Ref { is_mut: false, .. } | InferTyKind::Ptr { .. } | InferTyKind::FnPtr { .. } | InferTyKind::FnDef { .. })
                {
                    return true;
                }
            }
            Some(l) if l.is_fn_trait() => {
                let mut fn_cur = cur;
                while let InferTyKind::Ref { inner, .. } = arena.kind(fn_cur) {
                    fn_cur = vars.shallow_resolve(arena, inner);
                }
                match arena.kind(fn_cur) {
                    InferTyKind::Closure { sig, .. } | InferTyKind::FnPtr { sig } if sig.len > 0 => {
                        if let Some(&a0) = trait_args.first() {
                            if let InferTyKind::Tuple(r) = arena.kind(vars.shallow_resolve(arena, a0)) {
                                if r.len + 1 != sig.len {
                                    return false;
                                }
                            }
                            let mut sb = InlineTyBuf::new();
                            for &x in arena.slice(sig) {
                                sb.push(x);
                            }
                            let sl = sb.as_slice();
                            let tup = arena.mk_tuple(&sl[..sl.len() - 1]);
                            return unify::equate(arena, vars, a0, tup).is_ok();
                        }
                        return true;
                    }
                    InferTyKind::FnDef { def, args } => {
                        if let Some(&a0) = trait_args.first() {
                            let mut fargs = InlineTyBuf::new();
                            for &x in arena.slice(args) {
                                fargs.push(x);
                            }
                            return self.unify_fndef_inputs(def, fargs.as_slice(), a0, arena, vars);
                        }
                        return true;
                    }
                    _ => {}
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
        let concrete = self.ty_tables.concrete_impls(&(trait_gid, head));
        let blanket = self.ty_tables.blanket_impls(&trait_gid);

        let concrete_fn =
            matches!(arena.kind(cur), InferTyKind::FnDef { .. } | InferTyKind::Closure { .. } | InferTyKind::FnPtr { .. }) && !vars_has_unresolved_infer(arena, vars, cur);
        let has_infer = !concrete_fn && (vars_has_unresolved_infer(arena, vars, cur) || trait_args.iter().any(|&t| vars_has_unresolved_infer(arena, vars, t)));
        let total_cands = concrete.len() + blanket.len();
        let mut match_count = 0usize;
        let mut unique_imp: Option<TraitImplInfo> = None;
        for (i, &imp) in concrete.iter().chain(blanket.iter()).enumerate() {
            let snap = vars.start_snapshot();
            let ok = self.try_bind_v5_trait_impl(&imp, cur, trait_args, arena, vars, depth);
            if ok && (!has_infer || (match_count == 0 && i + 1 == total_cands)) {
                vars.commit(snap);
                return true;
            }
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
}
