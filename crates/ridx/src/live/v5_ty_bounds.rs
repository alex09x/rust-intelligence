/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Active generic parameter bound solving, supertrait argument unification,
//! trait-impl binding probes, and inherent `impl` specialization refinement on [`RidxV5ItemDb`].

use ridx_format::{DefKind, TyKindView, WherePredKind};
use ridx_syntax::name_res::{CrateId, GlobalDefId, ImplTargetKey, PrimTy};
use ridx_syntax::ty_infer::uf_table::InlineTyBuf;
use ridx_syntax::ty_infer::{unify, InferTyArena, InferTyId, InferTyKind, TyVarKind, UnionFindTable};

use super::v5_db::{RidxV5ItemDb, IMPL_SYNTH_BIT};
use super::v5_ty_lower::{lower_generic_args, lower_v5_ty, ParamRemap};
use super::v5_ty_proj::instantiate_v5;
use super::v5_ty_tables::TraitImplInfo;

impl<'w> RidxV5ItemDb<'w> {
    pub(crate) fn v5_param_trait_bounds(&self, raw_owner: GlobalDefId, param_idx: u16, out: &mut Vec<GlobalDefId>) {
        let owner = self.canonical_def(raw_owner);
        let r_idx = owner.krate.0 as usize;
        let Some(reader) = self.ws.readers().get(r_idx) else { return };
        let va = reader.ty_arena();
        for wp in self.iter_owner_where_preds(r_idx, owner.index) {
            if wp.kind() != WherePredKind::TraitBound || wp.is_negative() {
                continue;
            }
            if let TyKindView::Param { index, .. } = va.kind(wp.lhs_ty()) {
                if self.raw_to_dense_param_idx(r_idx, owner.index, index) == Some(param_idx) {
                    if let Some(td) = wp.trait_def() {
                        let tgid = self.ty_def_gid(r_idx, td);
                        if tgid.is_some() && !out.contains(&tgid) {
                            out.push(tgid);
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn try_bind_active_param_trait_args(&self, param_idx: u16, trait_gid: GlobalDefId, trait_args: &[InferTyId], arena: &mut InferTyArena, vars: &mut UnionFindTable) {
        let owner = self.canonical_def(self.active_owner_def.get());
        if !owner.is_some() {
            return;
        }
        let r_idx = owner.krate.0 as usize;
        let Some(reader) = self.ws.readers().get(r_idx) else { return };
        if param_idx == 0 {
            if let Some(&tr_didx) = self.shards.get(r_idx).and_then(|s| s.trait_parent_of_assoc.get(&owner.index)) {
                let parent_tr = self.canonical_def(GlobalDefId::new(owner.krate, tr_didx));
                if parent_tr == trait_gid {
                    for (i, &ta) in trait_args.iter().enumerate() {
                        let pty = arena.mk_param(1 + i as u16, GlobalDefId::NONE);
                        let _ = unify::equate(arena, vars, ta, pty);
                    }
                    return;
                }
            }
        }
        let remap = ParamRemap::for_def(self, r_idx, owner.index, false, arena);
        let va = reader.ty_arena();
        for wp in self.iter_owner_where_preds(r_idx, owner.index) {
            if wp.kind() != WherePredKind::TraitBound || wp.is_negative() {
                continue;
            }
            let TyKindView::Param { index, .. } = va.kind(wp.lhs_ty()) else { continue };
            if remap.dense_idx_of(index, arena) != Some(param_idx) {
                continue;
            }
            let Some(td) = wp.trait_def() else { continue };
            let b_tr = self.canonical_def(self.ty_def_gid(r_idx, td));
            let mut b_args = InlineTyBuf::new();
            lower_generic_args(self, r_idx, va, wp.trait_args(), &remap, arena, &mut b_args, 1);
            if b_tr == trait_gid {
                for (&ta, &ba) in trait_args.iter().zip(b_args.as_slice()) {
                    let _ = unify::equate(arena, vars, ta, ba);
                }
                return;
            }
            if self.trait_supertraits.get(&b_tr).is_some_and(|s| s.iter().any(|&x| self.canonical_def(x) == trait_gid)) {
                self.equate_supertrait_args(b_tr, trait_gid, arena.mk_param(param_idx, GlobalDefId::NONE), b_args.as_slice(), trait_args, arena, vars);
                return;
            }
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn equate_supertrait_args(
        &self,
        b_tr: GlobalDefId,
        trait_gid: GlobalDefId,
        self_ty: InferTyId,
        b_args: &[InferTyId],
        trait_args: &[InferTyId],
        arena: &mut InferTyArena,
        vars: &mut UnionFindTable,
    ) {
        let sr_idx = b_tr.krate.0 as usize;
        let Some(srdr) = self.ws.readers().get(sr_idx) else { return };
        let sremap = ParamRemap::for_def(self, sr_idx, b_tr.index, false, arena);
        let sva = srdr.ty_arena();
        let mut tr_subst = InlineTyBuf::new();
        tr_subst.push(self_ty);
        for &ba in b_args {
            tr_subst.push(ba);
        }
        let tgc = self.ty_tables.trait_non_lt_gc.get(&b_tr).copied().unwrap_or(0) as usize;
        for idx in b_args.len()..tgc {
            if let Some(dty) = self.v5_generic_param_default_ty(b_tr, idx as u16, arena) {
                tr_subst.push(instantiate_v5(arena, dty, tr_subst.as_slice(), 0));
            }
        }
        for swp in srdr.where_preds_for_def(b_tr.index) {
            if swp.kind() == WherePredKind::TraitBound && swp.trait_def().is_some_and(|std| self.canonical_def(self.ty_def_gid(sr_idx, std)) == trait_gid) {
                let mut s_args = InlineTyBuf::new();
                lower_generic_args(self, sr_idx, sva, swp.trait_args(), &sremap, arena, &mut s_args, 1);
                for (&ta, &sa) in trait_args.iter().zip(s_args.as_slice()) {
                    let inst_sa = instantiate_v5(arena, sa, tr_subst.as_slice(), 0);
                    let norm_sa = if let InferTyKind::Alias { trait_def: a_tr, assoc_def: a_ad, self_ty: a_st, args: a_args } = arena.kind(inst_sa) {
                        if let InferTyKind::Param { idx: p_idx, .. } = arena.kind(a_st) {
                            let mut ab = InlineTyBuf::new();
                            for &x in arena.slice(a_args) {
                                ab.push(x);
                            }
                            self.v5_project_param_assoc(self.active_owner_def.get(), p_idx, a_tr, a_ad, ab.as_slice(), arena).unwrap_or(inst_sa)
                        } else {
                            self.norm_v5_top_alias(inst_sa, arena, vars, 0)
                        }
                    } else {
                        inst_sa
                    };
                    let _ = unify::equate(arena, vars, ta, norm_sa);
                }
                return;
            }
        }
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
        self.active_owner_def.set(owner);
        if !raw_trait.is_some() && !raw_assoc.is_some() {
            return None;
        }
        let tgid = self.canonical_def(raw_trait);
        let agid = self.canonical_def(raw_assoc);
        let aname = *self.ty_tables.assoc_ty_names.get(&agid)?;
        let r_idx = owner.krate.0 as usize;
        let reader = self.ws.readers().get(r_idx)?;
        let remap = ParamRemap::for_def(self, r_idx, owner.index, false, arena);
        let va = reader.ty_arena();
        let (target_lhs, real_targs) = if param_idx == u16::MAX { (trait_args.first().copied(), trait_args.get(1..).unwrap_or(&[])) } else { (None, trait_args) };
        for wp in self.iter_owner_where_preds(r_idx, owner.index) {
            if wp.kind() != WherePredKind::ProjectionEq || wp.assoc_name() != Some(aname) {
                continue;
            }
            if let Some(tlhs) = target_lhs {
                if lower_v5_ty(self, r_idx, va, wp.lhs_ty(), &remap, arena) != tlhs {
                    continue;
                }
            } else {
                let TyKindView::Param { index, .. } = va.kind(wp.lhs_ty()) else { continue };
                if remap.dense_idx_of(index, arena) != Some(param_idx) {
                    continue;
                }
            }
            if tgid.is_some() {
                let Some(td) = wp.trait_def() else { continue };
                let b_tr = self.ty_def_gid(r_idx, td);
                if b_tr != tgid && !self.trait_supertraits.get(&b_tr).is_some_and(|s| s.iter().any(|&x| self.canonical_def(x) == tgid)) {
                    continue;
                }
            }
            if !real_targs.is_empty() {
                let mut wp_targs = InlineTyBuf::new();
                lower_generic_args(self, r_idx, va, wp.trait_args(), &remap, arena, &mut wp_targs, 1);
                if wp_targs.as_slice() != real_targs {
                    continue;
                }
            }
            if let Some(rhs) = wp.rhs_ty() {
                return Some(lower_v5_ty(self, r_idx, va, rhs, &remap, arena));
            }
        }
        None
    }

    pub(crate) fn callable_input_arity(&self, mut ty: InferTyId, arena: &InferTyArena) -> Option<u8> {
        while let InferTyKind::Ref { inner, .. } = arena.kind(ty) {
            ty = inner;
        }
        match arena.kind(ty) {
            InferTyKind::Closure { sig, .. } | InferTyKind::FnPtr { sig } if sig.len >= 1 => Some((sig.len - 1) as u8),
            InferTyKind::FnDef { def, .. } => {
                let cdef = self.canonical_def(def);
                let Some(reader) = self.ws.readers().get(cdef.krate.0 as usize) else {
                    return arena.overlay_fn_sig(def).filter(|s| s.len >= 1).map(|s| (s.len - 1) as u8);
                };
                if let Some(sig) = reader.fn_sig_for_def(cdef.index) {
                    return Some(sig.inputs().len() as u8);
                }
                reader.def_ty_for_def(cdef.index).filter(|d| d.ctor_shape() == ridx_format::CtorShape::Tuple).map(|_| reader.fields_for_owner(cdef.index).len() as u8)
            }
            InferTyKind::Prim(_) | InferTyKind::Tuple(_) | InferTyKind::Array { .. } | InferTyKind::Slice(_) | InferTyKind::Ptr { .. } | InferTyKind::Never => Some(254),
            InferTyKind::Adt { def, .. } if !self.metas.get(def.krate.0 as usize).is_some_and(|m| m.is_sysroot) => Some(254),
            _ => None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn try_bind_v5_trait_impl(
        &self,
        imp: &TraitImplInfo,
        cur: InferTyId,
        trait_args: &[InferTyId],
        arena: &mut InferTyArena,
        vars: &mut UnionFindTable,
        depth: u8,
    ) -> bool {
        if imp.fn_bound_arity != u8::MAX && self.callable_input_arity(cur, arena).is_some_and(|a| a != imp.fn_bound_arity) {
            return false;
        }
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
        if let Some((pi, clen)) = arena.extract_array_const_param(raw_self, cur) {
            subst.set(pi, arena.mk_array(InferTyId::NEVER, clen));
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
                let raw_ta = raw_targs.as_slice()[i];
                if !arena.flags(raw_ta).has_alias() {
                    let ia = instantiate_v5(arena, raw_ta, subst.as_slice(), 0);
                    if unify::equate(arena, vars, ta, ia).is_err() {
                        return false;
                    }
                }
            }
        }
        let bounds_ok = (imp.impl_gc == 0 && !imp.is_blanket) || self.check_impl_where_bounds_ext(r_idx, imp.impl_idx, &remap, subst.as_slice(), arena, vars, false, depth + 1);
        if !bounds_ok {
            return false;
        }
        if !trait_args.is_empty() {
            for (i, &ta) in trait_args.iter().enumerate() {
                let raw_ta = raw_targs.as_slice()[i];
                if arena.flags(raw_ta).has_alias() {
                    let ia = instantiate_v5(arena, raw_ta, subst.as_slice(), 0);
                    let norm_ia = self.norm_v5_top_alias(ia, arena, vars, depth + 1);
                    if unify::equate(arena, vars, ta, norm_ia).is_err() {
                        return false;
                    }
                }
            }
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn try_bind_unique_prim_trait_impl(
        &self,
        trait_gid: GlobalDefId,
        cur: InferTyId,
        trait_args: &[InferTyId],
        prims: &[PrimTy],
        arena: &mut InferTyArena,
        vars: &mut UnionFindTable,
        depth: u8,
    ) {
        let mut match_count = 0usize;
        let mut unique_imp: Option<(PrimTy, TraitImplInfo)> = None;
        for &prim in prims {
            let Some(impls) = self.ty_tables.concrete_trait_impls.get(&(trait_gid, ImplTargetKey::Prim(prim))) else { continue };
            for &imp in impls {
                let snap = vars.start_snapshot();
                let ok = self.try_bind_v5_trait_impl(&imp, InferTyId::from_prim(prim), trait_args, arena, vars, depth);
                vars.rollback_to(snap);
                if ok {
                    match_count += 1;
                    if match_count == 1 {
                        unique_imp = Some((prim, imp));
                    } else {
                        return;
                    }
                }
            }
        }
        if match_count == 0 && !trait_args.is_empty() && prims.len() <= 12 {
            if let Some(blankets) = self.ty_tables.blanket_trait_impls.get(&trait_gid).filter(|b| b.len() <= 4) {
                for &prim in prims {
                    for &imp in blankets {
                        let snap = vars.start_snapshot();
                        let ok = self.try_bind_v5_trait_impl(&imp, InferTyId::from_prim(prim), trait_args, arena, vars, depth);
                        vars.rollback_to(snap);
                        if ok {
                            match_count += 1;
                            if match_count == 1 {
                                unique_imp = Some((prim, imp));
                            } else {
                                return;
                            }
                        }
                    }
                }
            }
        }
        if match_count == 1 {
            if let Some((prim, imp)) = unique_imp {
                let snap = vars.start_snapshot();
                if unify::equate(arena, vars, cur, InferTyId::from_prim(prim)).is_ok() && self.try_bind_v5_trait_impl(&imp, cur, trait_args, arena, vars, depth) {
                    vars.commit(snap);
                } else {
                    vars.rollback_to(snap);
                }
            }
        }
    }

    pub fn v5_refine_inherent_assoc_fn(&self, assoc_fn: GlobalDefId, qself_ty: InferTyId, arena: &mut InferTyArena, vars: &mut UnionFindTable) -> GlobalDefId {
        let cur = vars.shallow_resolve(arena, qself_ty);
        let InferTyKind::Adt { def: adt_def, args } = arena.kind(cur) else { return assoc_fn };
        if args.len == 0 {
            return assoc_fn;
        }
        let fn_gid = self.canonical_def(assoc_fn);
        let Some(&impl_idx) = self.ty_tables.assoc_to_impl.get(&fn_gid) else { return assoc_fn };
        if self.probe_inherent_impl_self(fn_gid.krate.0 as usize, impl_idx, cur, arena, vars) {
            return assoc_fn;
        }
        let reader = &self.ws.readers()[fn_gid.krate.0 as usize];
        let Some(ad) = reader.definitions().get(fn_gid.index as usize) else { return assoc_fn };
        let fn_name = reader.get_str(ad.name_offset()).trim_start_matches("r#");
        let Some(impls) = self.impls_by_adt.get(&self.canonical_def(adt_def)) else { return assoc_fn };
        for imp in impls.iter().filter(|i| !i.trait_def.is_some()) {
            let r_idx = imp.id.krate.0 as usize;
            let rdr = &self.ws.readers()[r_idx];
            let Some(iv_idx) =
                (if (imp.id.index & IMPL_SYNTH_BIT) != 0 { Some(imp.id.index & !IMPL_SYNTH_BIT) } else { self.shards[r_idx].impl_def_to_impl_idx.get(&imp.id.index).copied() })
            else {
                continue;
            };
            let Some(iv) = rdr.impl_view(iv_idx) else { continue };
            if let Some(mdidx) = iv.assoc_def_indices().find(|&di| {
                rdr.definitions().get(di as usize).is_some_and(|d| DefKind::from(d.kind) == DefKind::AssocFn && rdr.get_str(d.name_offset()).trim_start_matches("r#") == fn_name)
            }) {
                if self.probe_inherent_impl_self(r_idx, iv_idx, cur, arena, vars) {
                    return GlobalDefId::new(CrateId(r_idx as u32), mdidx);
                }
            }
        }
        assoc_fn
    }

    fn probe_inherent_impl_self(&self, r_idx: usize, impl_idx: u32, cur: InferTyId, arena: &mut InferTyArena, vars: &mut UnionFindTable) -> bool {
        let reader = &self.ws.readers()[r_idx];
        let Some(iv) = reader.impl_view(impl_idx) else { return false };
        let remap = ParamRemap::for_impl(self, r_idx, impl_idx, arena);
        let raw_self = lower_v5_ty(self, r_idx, reader.ty_arena(), iv.self_ty(), &remap, arena);
        let snap = vars.start_snapshot();
        let mut subst = InlineTyBuf::new();
        for _ in 0..remap.non_lt_count {
            subst.push(vars.new_var(arena, TyVarKind::General).1);
        }
        let inst_self = instantiate_v5(arena, raw_self, subst.as_slice(), 0);
        let ok = unify::equate(arena, vars, cur, inst_self).is_ok();
        vars.rollback_to(snap);
        ok
    }
}
