/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Zero-allocation `.ridx` `v5` associated-type projection, adapter `ProjectionEq` evaluation,
//! `impl` compatibility probing, and parameter index remapping on [`RidxV5ItemDb`].

use ridx_format::{GenericParamKind, WherePredIter, WherePredKind, WherePredView};
use ridx_syntax::name_res::{CrateId, GlobalDefId, ImplTargetKey};
use ridx_syntax::ty_infer::autoderef::impl_target_key_of_ty;
use ridx_syntax::ty_infer::uf_table::InlineTyBuf;
use ridx_syntax::ty_infer::{unify, InferTyArena, InferTyId, InferTyKind, TyDefDb, UnionFindTable};

use crate::v5_db::RidxV5ItemDb;
use crate::v5_ty_lower::{lower_generic_args, lower_v5_ty, ParamRemap};
use crate::v5_ty_tables::{fnv1a_str, TraitImplInfo};

pub struct ChainedWherePreds<'a> {
    first: Option<WherePredIter<'a>>,
    second: WherePredIter<'a>,
}

impl<'a> Iterator for ChainedWherePreds<'a> {
    type Item = WherePredView<'a>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(it) = self.first.as_mut() {
            if let Some(v) = it.next() {
                return Some(v);
            }
            self.first = None;
        }
        self.second.next()
    }
}

impl<'w> RidxV5ItemDb<'w> {
    pub fn iter_owner_where_preds(&self, r_idx: usize, didx: u32) -> ChainedWherePreds<'_> {
        let reader = &self.ws.readers()[r_idx];
        let gid = GlobalDefId::new(CrateId(r_idx as u32), didx);
        let first = if let Some(&impl_idx) = self.ty_tables.assoc_to_impl.get(&gid) {
            Some(reader.where_preds_for_impl(impl_idx))
        } else {
            self.shards[r_idx].trait_parent_of_assoc.get(&didx).map(|&tdidx| reader.where_preds_for_def(tdidx))
        };
        ChainedWherePreds { first, second: reader.where_preds_for_def(didx) }
    }

    pub fn raw_to_dense_param_idx(&self, r_idx: usize, didx: u32, target_raw: u32) -> Option<u16> {
        let Ok(target16) = u16::try_from(target_raw) else {
            return None;
        };
        let reader = &self.ws.readers()[r_idx];
        let gid = GlobalDefId::new(CrateId(r_idx as u32), didx);
        let mut dense = 0u16;
        if let Some(&impl_idx) = self.ty_tables.assoc_to_impl.get(&gid) {
            for p in reader.generic_params_for_impl(impl_idx) {
                if matches!(p.kind, GenericParamKind::Lifetime) {
                    continue;
                }
                if p.param_idx == target16 {
                    return Some(dense);
                }
                dense = dense.saturating_add(1);
            }
        } else if let Some(&trait_didx) = self.shards[r_idx].trait_parent_of_assoc.get(&didx) {
            for p in reader.generic_params_for_def(trait_didx) {
                if matches!(p.kind, GenericParamKind::Lifetime) {
                    continue;
                }
                if p.param_idx == target16 {
                    return Some(dense);
                }
                dense = dense.saturating_add(1);
            }
        }
        for p in reader.generic_params_for_def(didx) {
            if matches!(p.kind, GenericParamKind::Lifetime) {
                continue;
            }
            if p.param_idx == target16 {
                return Some(dense);
            }
            dense = dense.saturating_add(1);
        }
        None
    }

    pub fn project_by_trait_and_name(
        &self,
        trait_gid: GlobalDefId,
        assoc_name: &str,
        raw_self: InferTyId,
        trait_args: &[InferTyId],
        arena: &mut InferTyArena,
        vars: &mut UnionFindTable,
    ) -> Option<InferTyId> {
        self.project_by_trait_and_name_depth(trait_gid, assoc_name, raw_self, trait_args, arena, vars, 0)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn project_by_trait_and_name_depth(
        &self,
        raw_trait_gid: GlobalDefId,
        assoc_name: &str,
        raw_self: InferTyId,
        trait_args: &[InferTyId],
        arena: &mut InferTyArena,
        vars: &mut UnionFindTable,
        depth: u8,
    ) -> Option<InferTyId> {
        if depth > 4 {
            return None;
        }
        let mut trait_gid = self.canonical_def(raw_trait_gid);
        let nh = fnv1a_str(assoc_name);
        if !self.ty_tables.trait_assoc_tys.get(&(trait_gid, nh)).is_some_and(|&(_, s, _)| s == assoc_name) && trait_args.is_empty() {
            if let Some(supers) = self.trait_supertraits.get(&trait_gid) {
                for &sup in supers {
                    let csup = self.canonical_def(sup);
                    if self.ty_tables.trait_assoc_tys.get(&(csup, nh)).is_some_and(|&(_, s, _)| s == assoc_name) {
                        trait_gid = csup;
                        break;
                    }
                }
            }
        }
        let self_ty = vars.shallow_resolve(arena, raw_self);
        if matches!(arena.kind(self_ty), InferTyKind::Unknown | InferTyKind::Infer(_) | InferTyKind::Param { .. } | InferTyKind::Alias { .. }) {
            return None;
        }
        if let InferTyKind::Opaque { def, args } | InferTyKind::Dyn { principal: def, args } = arena.kind(self_ty) {
            let cdef = self.canonical_def(def);
            if cdef == trait_gid {
                let tgc = self.ty_tables.trait_non_lt_gc.get(&trait_gid).copied().unwrap_or(0) as usize;
                let ord = self.ty_tables.trait_assoc_tys.get(&(trait_gid, nh)).map_or(0, |&(_, _, o)| o as usize);
                if let Some(&aty) = arena.slice(args).get(tgc + ord) {
                    return Some(vars.shallow_resolve(arena, aty));
                }
            }
        }

        let raw_head = impl_target_key_of_ty(arena, self_ty);
        let head = match raw_head {
            ImplTargetKey::Adt(d) => ImplTargetKey::Adt(self.canonical_def(d)),
            ImplTargetKey::Dyn(d) => ImplTargetKey::Dyn(self.canonical_def(d)),
            other => other,
        };
        let concrete = self.ty_tables.concrete_trait_impls.get(&(trait_gid, head)).map_or(&[][..], |v| v.as_slice());
        let blanket = self.ty_tables.blanket_trait_impls.get(&trait_gid).map_or(&[][..], |v| v.as_slice());
        let args_have_infer = trait_args.iter().any(|&a| {
            let ra = vars.shallow_resolve(arena, a);
            matches!(arena.kind(ra), InferTyKind::Infer(_))
        });
        if args_have_infer && concrete.len() > 1 {
            let mut self_matches = 0u8;
            for imp in concrete {
                let r_idx = imp.r_idx as usize;
                let reader = &self.ws.readers()[r_idx];
                let Some(iv) = reader.impl_view(imp.impl_idx) else { continue };
                let remap = ParamRemap::for_impl(self, r_idx, imp.impl_idx, arena);
                let raw_imp_self = lower_v5_ty(self, r_idx, reader.ty_arena(), iv.self_ty(), &remap, arena);
                let snap = vars.start_snapshot();
                let mut subst = InlineTyBuf::new();
                for _ in 0..imp.impl_gc {
                    subst.push(vars.new_var(arena, ridx_syntax::ty_infer::TyVarKind::General).1);
                }
                let inst_self = instantiate_v5(arena, raw_imp_self, subst.as_slice(), 0);
                if unify::equate(arena, vars, self_ty, inst_self).is_ok() {
                    self_matches += 1;
                }
                vars.rollback_to(snap);
                if self_matches > 1 {
                    return None;
                }
            }
        }

        for imp in concrete.iter().chain(blanket.iter()) {
            let r_idx = imp.r_idx as usize;
            let reader = &self.ws.readers()[r_idx];
            let Some(iv) = reader.impl_view(imp.impl_idx) else { continue };
            let va = reader.ty_arena();
            let remap = ParamRemap::for_impl(self, r_idx, imp.impl_idx, arena);
            let raw_imp_self = lower_v5_ty(self, r_idx, va, iv.self_ty(), &remap, arena);
            let mut raw_imp_args = InlineTyBuf::new();
            lower_generic_args(self, r_idx, va, iv.trait_args(), &remap, arena, &mut raw_imp_args, 1);

            let snap = vars.start_snapshot();
            let mut subst = InlineTyBuf::new();
            for _ in 0..imp.impl_gc {
                subst.push(vars.new_var(arena, ridx_syntax::ty_infer::TyVarKind::General).1);
            }
            let inst_self = instantiate_v5(arena, raw_imp_self, subst.as_slice(), 0);
            let mut ok = unify::equate(arena, vars, self_ty, inst_self).is_ok();
            if ok && !trait_args.is_empty() && raw_imp_args.as_slice().len() >= trait_args.len() {
                for (i, &actual_arg) in trait_args.iter().enumerate() {
                    let cand_arg = instantiate_v5(arena, raw_imp_args.as_slice()[i], subst.as_slice(), 0);
                    if unify::coerce(arena, vars, actual_arg, cand_arg).is_err() {
                        ok = false;
                        break;
                    }
                }
            }
            if ok && imp.is_blanket && !self.check_impl_where_bounds_ext(r_idx, imp.impl_idx, &remap, subst.as_slice(), arena, vars, false, depth) {
                ok = false;
            }
            if ok {
                if subst.as_slice().iter().any(|&v| matches!(arena.kind(vars.shallow_resolve(arena, v)), InferTyKind::Infer(_))) {
                    self.eval_impl_proj_eqs(r_idx, imp.impl_idx, &remap, subst.as_slice(), arena, vars, depth + 1);
                }
                let rhs_opt = iv.find_assoc_ty(assoc_name).map(|tid| {
                    let rty = lower_v5_ty(self, r_idx, va, tid, &remap, arena);
                    instantiate_v5(arena, rty, subst.as_slice(), 0)
                });
                if let Some(resolved_rhs) = rhs_opt {
                    vars.commit(snap);
                    return Some(vars.shallow_resolve(arena, resolved_rhs));
                }
                if let Some(&(agid, _, _)) = self.ty_tables.trait_assoc_tys.get(&(trait_gid, nh)).filter(|&&(_, s, _)| s == assoc_name) {
                    let tr_r = agid.krate.0 as usize;
                    let tr_rdr = &self.ws.readers()[tr_r];
                    if let Some(dt) = tr_rdr.def_ty_for_def(agid.index).filter(|dt| dt.is_default_assoc_ty()) {
                        let tr_remap = ParamRemap::for_def(self, tr_r, trait_gid.index, false, arena);
                        let tpl = lower_v5_ty(self, tr_r, tr_rdr.ty_arena(), dt.ty(), &tr_remap, arena);
                        let mut tr_subst = InlineTyBuf::new();
                        tr_subst.push(inst_self);
                        for &ia in raw_imp_args.as_slice() {
                            tr_subst.push(instantiate_v5(arena, ia, subst.as_slice(), 0));
                        }
                        let inst_def = instantiate_v5(arena, tpl, tr_subst.as_slice(), 0);
                        vars.commit(snap);
                        return Some(vars.shallow_resolve(arena, inst_def));
                    }
                }
            }
            vars.rollback_to(snap);
        }
        None
    }

    #[allow(clippy::too_many_arguments)]
    fn eval_impl_proj_eqs(&self, r_idx: usize, impl_idx: u32, remap: &ParamRemap, subst: &[InferTyId], arena: &mut InferTyArena, vars: &mut UnionFindTable, depth: u8) {
        if depth > 4 {
            return;
        }
        let reader = &self.ws.readers()[r_idx];
        let va = reader.ty_arena();
        for wp in reader.where_preds_for_impl(impl_idx) {
            if wp.kind() != WherePredKind::ProjectionEq {
                continue;
            }
            let (Some(td), Some(aname), Some(rhs)) = (wp.trait_def(), wp.assoc_name(), wp.rhs_ty()) else { continue };
            let b_trait = self.ty_def_gid(r_idx, td);
            if !b_trait.is_some() {
                continue;
            }
            let raw_lhs = lower_v5_ty(self, r_idx, va, wp.lhs_ty(), remap, arena);
            let tmp_lhs = instantiate_v5(arena, raw_lhs, subst, 0);
            let inst_lhs = vars.shallow_resolve(arena, tmp_lhs);
            let raw_rhs = lower_v5_ty(self, r_idx, va, rhs, remap, arena);
            let inst_rhs = instantiate_v5(arena, raw_rhs, subst, 0);

            let proj_opt = if self.lang_item_of_def(b_trait).is_some_and(|l| l.is_fn_trait()) && aname == "Output" {
                match arena.kind(inst_lhs) {
                    InferTyKind::Closure { sig, .. } | InferTyKind::FnPtr { sig } if sig.len > 0 => Some(arena.slice(sig)[sig.len as usize - 1]),
                    InferTyKind::FnDef { def, args } => {
                        let mut fargs = InlineTyBuf::new();
                        for i in 0..args.len as usize {
                            fargs.push(arena.slice(args)[i]);
                        }
                        self.fn_output_only(def, arena).map(|out| instantiate_v5(arena, out, fargs.as_slice(), 0))
                    }
                    InferTyKind::Dyn { args, .. } | InferTyKind::Opaque { args, .. } if args.len >= 2 => Some(arena.slice(args)[1]),
                    _ => None,
                }
            } else {
                let mut targs = InlineTyBuf::new();
                lower_generic_args(self, r_idx, va, wp.trait_args(), remap, arena, &mut targs, 1);
                let mut inst_targs = InlineTyBuf::new();
                for &ta in targs.as_slice() {
                    inst_targs.push(instantiate_v5(arena, ta, subst, 0));
                }
                self.project_by_trait_and_name_depth(b_trait, aname, inst_lhs, inst_targs.as_slice(), arena, vars, depth + 1)
            };
            if let Some(proj_ty) = proj_opt {
                let _ = unify::equate(arena, vars, proj_ty, inst_rhs);
            }
        }
    }

    fn fn_output_only(&self, raw_def: GlobalDefId, arena: &mut InferTyArena) -> Option<InferTyId> {
        let def = self.canonical_def(raw_def);
        let r_idx = def.krate.0 as usize;
        let reader = self.ws.readers().get(r_idx)?;
        let sig = reader.fn_sig_for_def(def.index)?;
        let gen_owner = reader.def_ty_for_def(def.index).and_then(|dt| dt.parent_def_index()).unwrap_or(def.index);
        let remap = ParamRemap::for_def(self, r_idx, gen_owner, false, arena);
        Some(lower_v5_ty(self, r_idx, reader.ty_arena(), sig.output(), &remap, arena))
    }

    pub fn probe_impl_compat(&self, imp: &TraitImplInfo, raw_step_ty: InferTyId, arena: &mut InferTyArena, vars: &mut UnionFindTable) -> Option<(InferTyId, InlineTyBuf)> {
        let step_ty = vars.shallow_resolve(arena, raw_step_ty);
        if matches!(arena.kind(step_ty), InferTyKind::Unknown | InferTyKind::Infer(_)) {
            return None;
        }
        let r_idx = imp.r_idx as usize;
        let reader = &self.ws.readers()[r_idx];
        let iv = reader.impl_view(imp.impl_idx)?;
        let va = reader.ty_arena();
        let remap = ParamRemap::for_impl(self, r_idx, imp.impl_idx, arena);
        let raw_self = lower_v5_ty(self, r_idx, va, iv.self_ty(), &remap, arena);
        let mut raw_targs = InlineTyBuf::new();
        lower_generic_args(self, r_idx, va, iv.trait_args(), &remap, arena, &mut raw_targs, 1);

        let snap = vars.start_snapshot();
        let mut subst = InlineTyBuf::new();
        for _ in 0..imp.impl_gc {
            subst.push(vars.new_var(arena, ridx_syntax::ty_infer::TyVarKind::General).1);
        }
        let inst_self = instantiate_v5(arena, raw_self, subst.as_slice(), 0);
        let ok = unify::equate(arena, vars, inst_self, step_ty).is_ok()
            && (!imp.is_blanket || self.check_impl_where_bounds_ext(r_idx, imp.impl_idx, &remap, subst.as_slice(), arena, vars, true, 0));
        vars.rollback_to(snap);
        ok.then_some((raw_self, raw_targs))
    }
}

pub fn instantiate_v5(arena: &mut InferTyArena, ty: InferTyId, subst: &[InferTyId], depth: u8) -> InferTyId {
    if depth > 48 || !arena.flags(ty).has_param() {
        return ty;
    }
    match arena.kind(ty) {
        InferTyKind::Param { idx, .. } => subst.get(idx as usize).copied().unwrap_or(InferTyId::UNKNOWN),
        InferTyKind::Ref { is_mut, inner } => {
            let s = instantiate_v5(arena, inner, subst, depth + 1);
            arena.mk_ref(is_mut, s)
        }
        InferTyKind::Ptr { is_mut, inner } => {
            let s = instantiate_v5(arena, inner, subst, depth + 1);
            arena.mk_ptr(is_mut, s)
        }
        InferTyKind::Slice(e) => {
            let s = instantiate_v5(arena, e, subst, depth + 1);
            arena.mk_slice(s)
        }
        InferTyKind::Array { elem, len } => {
            let s = instantiate_v5(arena, elem, subst, depth + 1);
            arena.mk_array(s, len)
        }
        InferTyKind::Tuple(r) => {
            let b = inst_slice_v5(arena, r, subst, depth + 1);
            arena.mk_tuple(b.as_slice())
        }
        InferTyKind::Adt { def, args } => {
            let b = inst_slice_v5(arena, args, subst, depth + 1);
            arena.mk_adt(def, b.as_slice())
        }
        InferTyKind::FnPtr { sig } => {
            let b = inst_slice_v5(arena, sig, subst, depth + 1);
            let s = b.as_slice();
            arena.mk_fn_ptr(&s[..s.len() - 1], s[s.len() - 1])
        }
        InferTyKind::FnDef { def, args } => {
            let b = inst_slice_v5(arena, args, subst, depth + 1);
            arena.mk_fn_def(def, b.as_slice())
        }
        InferTyKind::Alias { trait_def, assoc_def, self_ty, args } => {
            let ns = instantiate_v5(arena, self_ty, subst, depth + 1);
            let b = inst_slice_v5(arena, args, subst, depth + 1);
            arena.mk_alias(trait_def, assoc_def, ns, b.as_slice())
        }
        InferTyKind::Dyn { principal, args } => {
            let b = inst_slice_v5(arena, args, subst, depth + 1);
            arena.mk_dyn(principal, b.as_slice())
        }
        InferTyKind::Opaque { def, args } => {
            let b = inst_slice_v5(arena, args, subst, depth + 1);
            arena.mk_opaque(def, b.as_slice())
        }
        _ => ty,
    }
}

fn inst_slice_v5(arena: &mut InferTyArena, r: ridx_syntax::ty_infer::TySliceRange, subst: &[InferTyId], depth: u8) -> InlineTyBuf {
    let mut b = InlineTyBuf::new();
    for idx in 0..r.len as usize {
        b.push(instantiate_v5(arena, arena.slice(r)[idx], subst, depth));
    }
    b
}
