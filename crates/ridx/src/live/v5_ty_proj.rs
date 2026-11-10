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

use ridx_format::{GenericParamKind, WherePredIter, WherePredView};
use ridx_syntax::name_res::{CrateId, GlobalDefId, ImplTargetKey};
use ridx_syntax::ty_infer::autoderef::impl_target_key_of_ty;
use ridx_syntax::ty_infer::uf_table::InlineTyBuf;
use ridx_syntax::ty_infer::{unify, InferTyArena, InferTyId, InferTyKind, TyDefDb, UnionFindTable};

use super::v5_db::RidxV5ItemDb;
use super::v5_ty_lower::{lower_generic_args, lower_v5_ty, ParamRemap};
use super::v5_ty_tables::{fnv1a_str, TraitImplInfo};

pub struct ChainedWherePreds<'a> {
    first: Option<WherePredIter<'a>>,
    second: WherePredIter<'a>,
}

impl<'a> Iterator for ChainedWherePreds<'a> {
    type Item = WherePredView<'a>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(v) = self.first.as_mut().and_then(Iterator::next) {
            return Some(v);
        }
        self.first = None;
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
        let target16 = u16::try_from(target_raw).ok()?;
        let reader = &self.ws.readers()[r_idx];
        let gid = GlobalDefId::new(CrateId(r_idx as u32), didx);
        let mut dense = 0u16;
        let scan_iter = |it: ridx_format::GenericParamIter<'_>, dense: &mut u16| -> Option<u16> {
            for p in it.filter(|p| !matches!(p.kind, GenericParamKind::Lifetime)) {
                if p.param_idx == target16 {
                    return Some(*dense);
                }
                *dense = dense.saturating_add(1);
            }
            None
        };
        if let Some(&impl_idx) = self.ty_tables.assoc_to_impl.get(&gid) {
            if let Some(d) = scan_iter(reader.generic_params_for_impl(impl_idx), &mut dense) {
                return Some(d);
            }
        } else if let Some(&trait_didx) = self.shards[r_idx].trait_parent_of_assoc.get(&didx) {
            if let Some(d) = scan_iter(reader.generic_params_for_def(trait_didx), &mut dense) {
                return Some(d);
            }
        }
        scan_iter(reader.generic_params_for_def(didx), &mut dense)
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
        if !self.ty_tables.trait_assoc_tys.get(&(trait_gid, nh)).is_some_and(|&(_, s, _)| s.as_str(self.ws) == assoc_name) && trait_args.is_empty() {
            if let Some(supers) = self.trait_supertraits.get(&trait_gid) {
                for &sup in supers {
                    let csup = self.canonical_def(sup);
                    if self.ty_tables.trait_assoc_tys.get(&(csup, nh)).is_some_and(|&(_, s, _)| s.as_str(self.ws) == assoc_name) {
                        trait_gid = csup;
                        break;
                    }
                }
            }
        }
        let self_ty = vars.shallow_resolve(arena, raw_self);
        if let InferTyKind::Infer(v) = arena.kind(self_ty) {
            if let Err((_, ridx_syntax::ty_infer::TyVarKind::IntVar | ridx_syntax::ty_infer::TyVarKind::FloatVar, ..)) = vars.probe_var(v) {
                let _ = self.v5_solve_trait_obligation(trait_gid, self_ty, trait_args, arena, vars);
            }
        }
        let self_ty = vars.shallow_resolve(arena, self_ty);
        if self.lang_item_of_def(trait_gid).is_some_and(|l| l.is_fn_trait()) && assoc_name == "Output" {
            let mut fn_self = self_ty;
            while let InferTyKind::Ref { inner, .. } = arena.kind(fn_self) {
                fn_self = vars.shallow_resolve(arena, inner);
            }
            match arena.kind(fn_self) {
                InferTyKind::Closure { sig, .. } | InferTyKind::FnPtr { sig } if sig.len > 0 => {
                    let mut sb = InlineTyBuf::new();
                    for &x in arena.slice(sig) {
                        sb.push(x);
                    }
                    let sl = sb.as_slice();
                    if let Some(&a0) = trait_args.first() {
                        let tup = arena.mk_tuple(&sl[..sl.len() - 1]);
                        let _ = unify::equate(arena, vars, a0, tup);
                    }
                    return Some(vars.shallow_resolve(arena, sl[sl.len() - 1]));
                }
                InferTyKind::FnDef { def, args } => {
                    let mut fargs = InlineTyBuf::new();
                    for &x in arena.slice(args) {
                        fargs.push(x);
                    }
                    if let Some(&a0) = trait_args.first() {
                        self.unify_fndef_inputs(def, fargs.as_slice(), a0, arena, vars);
                    }
                    if let Some(out) = self.fn_output_only(def, arena) {
                        let inst_out = instantiate_v5(arena, out, fargs.as_slice(), 0);
                        return Some(vars.shallow_resolve(arena, inst_out));
                    }
                }
                InferTyKind::Dyn { args, .. } | InferTyKind::Opaque { args, .. } if args.len >= 2 => {
                    return Some(vars.shallow_resolve(arena, arena.slice(args)[1]));
                }
                _ => {}
            }
        }
        if let InferTyKind::Alias { assoc_def: inner_assoc, self_ty: inner_self, args: inner_args, .. } = arena.kind(self_ty) {
            let b = self.project_alias_item_bound(inner_assoc, inner_self, inner_args, trait_gid, assoc_name, arena)?;
            if let InferTyKind::Alias { trait_def: b_tr, assoc_def: b_ad, self_ty: b_st, .. } = arena.kind(b) {
                if let Some(&b_name) = self.ty_tables.assoc_ty_names.get(&self.canonical_def(b_ad)) {
                    if let Some(norm_b) = self.project_by_trait_and_name_depth(b_tr, b_name.as_str(self.ws), b_st, &[], arena, vars, depth + 1) {
                        return Some(norm_b);
                    }
                }
            }
            return Some(b);
        }
        if let InferTyKind::Param { idx, .. } = arena.kind(self_ty) {
            if !trait_args.is_empty() {
                self.try_bind_active_param_trait_args(idx, trait_gid, trait_args, arena, vars);
            }
            return None;
        }
        if matches!(arena.kind(self_ty), InferTyKind::Unknown | InferTyKind::Infer(_)) {
            return None;
        }
        if let InferTyKind::Opaque { def, args } | InferTyKind::Dyn { principal: def, args } = arena.kind(self_ty) {
            let cdef = self.canonical_def(def);
            if cdef == trait_gid || self.trait_supertraits.get(&cdef).is_some_and(|s| s.iter().any(|&x| self.canonical_def(x) == trait_gid)) {
                let tgc = self.ty_tables.trait_non_lt_gc.get(&cdef).copied().unwrap_or(0) as usize;
                let ord = self.ty_tables.trait_assoc_tys.get(&(trait_gid, nh)).map_or(0, |&(_, _, o)| o as usize);
                if let Some(&t) = arena.slice(args).get(tgc + ord) {
                    return Some(vars.shallow_resolve(arena, t));
                }
            }
            if matches!(arena.kind(self_ty), InferTyKind::Opaque { .. }) {
                return None;
            }
        }

        let raw_head = impl_target_key_of_ty(arena, self_ty);
        let head = match raw_head {
            ImplTargetKey::Adt(d) => ImplTargetKey::Adt(self.canonical_def(d)),
            ImplTargetKey::Dyn(d) => ImplTargetKey::Dyn(self.canonical_def(d)),
            other => other,
        };
        let concrete = self.ty_tables.concrete_impls(&(trait_gid, head));
        let blanket = self.ty_tables.blanket_impls(&trait_gid);
        let args_have_infer = trait_args.iter().any(|&a| matches!(arena.kind(vars.shallow_resolve(arena, a)), InferTyKind::Infer(_)));
        if args_have_infer && concrete.len() > 1 {
            let mut self_matches = 0u8;
            for imp in concrete {
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
                if ok && raw_imp_args.as_slice().len() >= trait_args.len() {
                    for (i, &actual_arg) in trait_args.iter().enumerate() {
                        let cand_arg = instantiate_v5(arena, raw_imp_args.as_slice()[i], subst.as_slice(), 0);
                        if unify::coerce(arena, vars, actual_arg, cand_arg).is_err() {
                            ok = false;
                            break;
                        }
                    }
                }
                vars.rollback_to(snap);
                if ok {
                    self_matches += 1;
                    if self_matches > 1 {
                        return None;
                    }
                }
            }
        }

        let cur_arity = self.callable_input_arity(self_ty, arena);
        for imp in concrete.iter().chain(blanket.iter()) {
            if imp.fn_bound_arity != u8::MAX && cur_arity.is_some_and(|a| a != imp.fn_bound_arity) {
                continue;
            }
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
            if let Some((pi, clen)) = arena.extract_array_const_param(raw_imp_self, self_ty) {
                subst.set(pi, arena.mk_array(InferTyId::NEVER, clen));
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
                if let Some(&(agid, _, _)) = self.ty_tables.trait_assoc_tys.get(&(trait_gid, nh)).filter(|&&(_, s, _)| s.as_str(self.ws) == assoc_name) {
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

    pub fn probe_impl_compat(&self, imp: &TraitImplInfo, raw_step_ty: InferTyId, arena: &mut InferTyArena, vars: &mut UnionFindTable) -> Option<(InferTyId, InlineTyBuf)> {
        let step_ty = vars.shallow_resolve(arena, raw_step_ty);
        if matches!(arena.kind(step_ty), InferTyKind::Unknown | InferTyKind::Infer(_))
            || (imp.fn_bound_arity != u8::MAX && self.callable_input_arity(step_ty, arena).is_some_and(|a| a != imp.fn_bound_arity))
        {
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

pub use super::v5_ty_inst::instantiate_v5;
