/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Default generic type parameter lowering (`T = Default`) and stdlib macro result type
//! construction (`StdMacroTy`) for [`super::v5_db::RidxV5ItemDb`] (`0` steady-state heap allocs).

use ridx_format::{GenericParamKind, TyKindView, WherePredKind};
use ridx_syntax::name_res::GlobalDefId;
use ridx_syntax::ty_infer::uf_table::InlineTyBuf;
use ridx_syntax::ty_infer::{unify, InferTyArena, InferTyId, StdMacroTy, TyDefDb, TyLangItem, TySliceRange, UnionFindTable};

use super::v5_db::{RidxV5ItemDb, CRATE_ROOT_DEF_IDX, IMPL_SYNTH_BIT, PROC_MACRO_SYNTH_BIT};
use super::v5_ty_lower::{lower_generic_args, lower_v5_ty, ParamRemap};
use super::v5_ty_proj::instantiate_v5;

impl RidxV5ItemDb<'_> {
    pub(crate) fn populate_v5_std_macro_gids(&mut self) {
        if self.ty_tables.vec_gid.is_none() {
            if let Some(&alloc_r) = self.sysroot_by_name.get("alloc").filter(|&&r| self.shard_loaded[r]) {
                let vg = self.find_def_in_reader_by_path(alloc_r, "alloc::vec::Vec");
                if vg.is_some() {
                    let c = self.canonical_def(vg);
                    self.state.as_mut().ty_tables.vec_gid = Some(c);
                }
                let gg = self.find_def_in_reader_by_path(alloc_r, "alloc::alloc::Global");
                if gg.is_some() {
                    let c = self.canonical_def(gg);
                    self.state.as_mut().ty_tables.global_gid = Some(c);
                }
            }
        }
        if self.ty_tables.fmt_args_gid.is_none() {
            if let Some(&core_r) = self.sysroot_by_name.get("core").filter(|&&r| self.shard_loaded[r]) {
                let ag = self.find_def_in_reader_by_path(core_r, "core::fmt::Arguments");
                let ag2 = if ag.is_some() { ag } else { self.find_def_in_reader_by_path(core_r, "core::fmt::rt::Arguments") };
                if ag2.is_some() {
                    let c = self.canonical_def(ag2);
                    self.state.as_mut().ty_tables.fmt_args_gid = Some(c);
                }
                let eg = self.find_def_in_reader_by_path(core_r, "core::fmt::Error");
                if eg.is_some() {
                    let c = self.canonical_def(eg);
                    self.state.as_mut().ty_tables.fmt_err_gid = Some(c);
                }
            }
        }
        if self.ty_tables.thread_local_key_gid.is_none() {
            if let Some(&std_r) = self.sysroot_by_name.get("std").filter(|&&r| self.shard_loaded[r]) {
                let tg = self.find_def_in_reader_by_path(std_r, "std::thread::local::LocalKey");
                let tg2 = if tg.is_some() { tg } else { self.find_def_in_reader_by_path(std_r, "std::thread::LocalKey") };
                if tg2.is_some() {
                    let c = self.canonical_def(tg2);
                    self.state.as_mut().ty_tables.thread_local_key_gid = Some(c);
                }
            }
        }
    }

    pub(crate) fn v5_generic_param_default_ty(&self, raw_def: GlobalDefId, param_idx: u16, arena: &mut InferTyArena) -> Option<InferTyId> {
        let def = self.canonical_def(raw_def);
        if !def.is_some() || def.index == CRATE_ROOT_DEF_IDX || (def.index & (IMPL_SYNTH_BIT | PROC_MACRO_SYNTH_BIT)) != 0 {
            return None;
        }
        let r_idx = def.krate.0 as usize;
        let reader = self.ws.readers().get(r_idx)?;
        let param = reader.generic_params_for_def(def.index).filter(|p| !matches!(p.kind, GenericParamKind::Lifetime) && !p.is_self()).nth(param_idx as usize)?;
        if matches!(param.kind, GenericParamKind::Const) && param.has_default() {
            return Some(arena.mk_array(InferTyId::NEVER, u64::MAX));
        }
        let raw_ty = param.default_ty()?;
        let remap = ParamRemap::for_def(self, r_idx, def.index, false, arena);
        Some(lower_v5_ty(self, r_idx, reader.ty_arena(), raw_ty, &remap, arena))
    }

    pub(crate) fn v5_std_macro_ty(&self, kind: StdMacroTy, arena: &mut InferTyArena) -> Option<InferTyId> {
        match kind {
            StdMacroTy::Vec(elem) => {
                let vd = self.ty_tables.vec_gid?;
                if let Some(gd) = self.ty_tables.global_gid {
                    let gty = arena.mk_adt(gd, &[]);
                    Some(arena.mk_adt(vd, &[elem, gty]))
                } else {
                    Some(arena.mk_adt(vd, &[elem]))
                }
            }
            StdMacroTy::String => self.lang_item(TyLangItem::String).map(|d| arena.mk_adt(d, &[])),
            StdMacroTy::FmtArguments => self.ty_tables.fmt_args_gid.map(|d| arena.mk_adt(d, &[])),
            StdMacroTy::FmtResult => {
                let res_d = self.lang_item(TyLangItem::Result)?;
                let err_ty = self.ty_tables.fmt_err_gid.map_or(InferTyId::UNKNOWN, |d| arena.mk_adt(d, &[]));
                Some(arena.mk_adt(res_d, &[InferTyId::UNIT, err_ty]))
            }
            StdMacroTy::OptionStr => self.lang_item(TyLangItem::Option).map(|d| arena.mk_adt(d, &[InferTyId::STR_REF])),
            StdMacroTy::ThreadLocalKey(inner) => self.ty_tables.thread_local_key_gid.map(|d| arena.mk_adt(d, &[inner])),
        }
    }

    pub(crate) fn v5_assoc_ty_parent_trait(&self, raw_assoc: GlobalDefId) -> Option<GlobalDefId> {
        let agid = self.canonical_def(raw_assoc);
        let r_idx = agid.krate.0 as usize;
        let &tr_didx = self.shards.get(r_idx)?.trait_parent_of_assoc.get(&agid.index)?;
        Some(self.canonical_def(GlobalDefId::new(agid.krate, tr_didx)))
    }

    pub(crate) fn v5_assoc_ty_trait_bounds(&self, raw_assoc: GlobalDefId, out: &mut Vec<GlobalDefId>) {
        let agid = self.canonical_def(raw_assoc);
        let r_idx = agid.krate.0 as usize;
        let Some(reader) = self.ws.readers().get(r_idx) else { return };
        let va = reader.ty_arena();
        for wp in reader.where_preds_for_def(agid.index) {
            if wp.kind() != WherePredKind::TraitBound || wp.is_negative() {
                continue;
            }
            if matches!(va.kind(wp.lhs_ty()), TyKindView::Assoc { .. }) {
                if let Some(td) = wp.trait_def() {
                    let tgid = self.ty_def_gid(r_idx, td);
                    if tgid.is_some() && !out.contains(&tgid) {
                        out.push(tgid);
                    }
                }
            }
        }
    }

    pub(crate) fn project_alias_item_bound(
        &self,
        raw_inner_assoc: GlobalDefId,
        inner_self: InferTyId,
        inner_args: TySliceRange,
        trait_gid: GlobalDefId,
        assoc_name: &str,
        arena: &mut InferTyArena,
    ) -> Option<InferTyId> {
        let agid = self.canonical_def(raw_inner_assoc);
        let r_idx = agid.krate.0 as usize;
        let reader = self.ws.readers().get(r_idx)?;
        let remap = ParamRemap::for_def(self, r_idx, agid.index, false, arena);
        let va = reader.ty_arena();
        for wp in reader.where_preds_for_def(agid.index) {
            if wp.kind() != WherePredKind::ProjectionEq || wp.assoc_name() != Some(assoc_name) {
                continue;
            }
            if !matches!(va.kind(wp.lhs_ty()), TyKindView::Assoc { .. }) {
                continue;
            }
            let Some(td) = wp.trait_def() else { continue };
            let b_tr = self.ty_def_gid(r_idx, td);
            let tr_ok = b_tr == trait_gid || self.trait_supertraits.get(&b_tr).is_some_and(|s| s.iter().any(|&x| self.canonical_def(x) == trait_gid));
            if !tr_ok {
                continue;
            }
            let rhs = wp.rhs_ty()?;
            let raw_rhs = lower_v5_ty(self, r_idx, va, rhs, &remap, arena);
            let mut subst = InlineTyBuf::new();
            subst.push(inner_self);
            for i in 0..inner_args.len as usize {
                subst.push(arena.slice(inner_args)[i]);
            }
            return Some(instantiate_v5(arena, raw_rhs, subst.as_slice(), 0));
        }
        None
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn eval_impl_proj_eqs(&self, r_idx: usize, impl_idx: u32, remap: &ParamRemap, subst: &[InferTyId], arena: &mut InferTyArena, vars: &mut UnionFindTable, depth: u8) {
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

            let mut targs = InlineTyBuf::new();
            lower_generic_args(self, r_idx, va, wp.trait_args(), remap, arena, &mut targs, 1);
            let mut inst_targs = InlineTyBuf::new();
            for &ta in targs.as_slice() {
                inst_targs.push(instantiate_v5(arena, ta, subst, 0));
            }
            if let Some(proj_ty) = self.project_by_trait_and_name_depth(b_trait, aname, inst_lhs, inst_targs.as_slice(), arena, vars, depth + 1) {
                let _ = unify::equate(arena, vars, proj_ty, inst_rhs);
            }
        }
    }

    pub(crate) fn fn_output_only(&self, raw_def: GlobalDefId, arena: &mut InferTyArena) -> Option<InferTyId> {
        let def = self.canonical_def(raw_def);
        let r_idx = def.krate.0 as usize;
        let Some(reader) = self.ws.readers().get(r_idx) else {
            return arena.overlay_fn_sig(raw_def).and_then(|sig| arena.slice(sig).last().copied());
        };
        if let Some(sig) = reader.fn_sig_for_def(def.index) {
            let gen_owner = reader.def_ty_for_def(def.index).and_then(|dt| dt.parent_def_index()).unwrap_or(def.index);
            let remap = ParamRemap::for_def(self, r_idx, gen_owner, false, arena);
            return Some(lower_v5_ty(self, r_idx, reader.ty_arena(), sig.output(), &remap, arena));
        }
        let dt = reader.def_ty_for_def(def.index).filter(|d| d.ctor_shape() == ridx_format::CtorShape::Tuple)?;
        let gen_owner = dt.parent_def_index().unwrap_or(def.index);
        let remap = ParamRemap::for_def(self, r_idx, gen_owner, false, arena);
        Some(lower_v5_ty(self, r_idx, reader.ty_arena(), dt.ty(), &remap, arena))
    }

    pub(crate) fn unify_fndef_inputs(&self, raw_def: GlobalDefId, fargs: &[InferTyId], arg_tup: InferTyId, arena: &mut InferTyArena, vars: &mut UnionFindTable) -> bool {
        let def = self.canonical_def(raw_def);
        let r_idx = def.krate.0 as usize;
        let Some(reader) = self.ws.readers().get(r_idx) else {
            let Some(sig) = arena.overlay_fn_sig(raw_def).filter(|s| s.len >= 1) else { return true };
            let mut raw_ins = InlineTyBuf::new();
            for &x in &arena.slice(sig)[..sig.len as usize - 1] {
                raw_ins.push(x);
            }
            let mut ins = InlineTyBuf::new();
            for &raw_in in raw_ins.as_slice() {
                ins.push(instantiate_v5(arena, raw_in, fargs, 0));
            }
            let in_tup = arena.mk_tuple(ins.as_slice());
            return unify::equate(arena, vars, arg_tup, in_tup).is_ok();
        };
        let resolved_tup = vars.shallow_resolve(arena, arg_tup);
        let expected_arity = if let ridx_syntax::ty_infer::InferTyKind::Tuple(r) = arena.kind(resolved_tup) { Some(r.len as usize) } else { None };
        let va = reader.ty_arena();
        let mut ins = InlineTyBuf::new();
        if let Some(sig) = reader.fn_sig_for_def(def.index) {
            if expected_arity.is_some_and(|n| n != sig.inputs().len()) {
                return false;
            }
            let gen_owner = reader.def_ty_for_def(def.index).and_then(|dt| dt.parent_def_index()).unwrap_or(def.index);
            let remap = ParamRemap::for_def(self, r_idx, gen_owner, false, arena);
            for inp in sig.inputs() {
                let raw_in = lower_v5_ty(self, r_idx, va, inp, &remap, arena);
                ins.push(instantiate_v5(arena, raw_in, fargs, 0));
            }
        } else if let Some(dt) = reader.def_ty_for_def(def.index).filter(|d| d.ctor_shape() == ridx_format::CtorShape::Tuple) {
            let gen_owner = dt.parent_def_index().unwrap_or(def.index);
            let remap = ParamRemap::for_def(self, r_idx, gen_owner, false, arena);
            for f in reader.fields_for_owner(def.index) {
                let raw_in = lower_v5_ty(self, r_idx, va, f.ty, &remap, arena);
                ins.push(instantiate_v5(arena, raw_in, fargs, 0));
            }
        } else {
            return true;
        }
        let in_tup = arena.mk_tuple(ins.as_slice());
        unify::equate(arena, vars, arg_tup, in_tup).is_ok()
    }

    pub(crate) fn norm_v5_top_alias(&self, ty: InferTyId, arena: &mut InferTyArena, vars: &mut UnionFindTable, depth: u8) -> InferTyId {
        let r = vars.shallow_resolve(arena, ty);
        if let ridx_syntax::ty_infer::InferTyKind::Alias { trait_def, assoc_def, self_ty, args } = arena.kind(r) {
            let mut targs = InlineTyBuf::new();
            for &x in arena.slice(args) {
                targs.push(vars.shallow_resolve(arena, x));
            }
            if let Some(&aname) = self.ty_tables.assoc_ty_names.get(&self.canonical_def(assoc_def)) {
                if let Some(p) = self.project_by_trait_and_name_depth(trait_def, aname.as_str(self.ws), self_ty, targs.as_slice(), arena, vars, depth) {
                    return p;
                }
            }
        }
        r
    }
}

pub const fn u16_to_ty_lang(raw: u16) -> Option<TyLangItem> {
    match raw {
        0 => Some(TyLangItem::Sized),
        1 => Some(TyLangItem::Copy),
        2 => Some(TyLangItem::Clone),
        3 => Some(TyLangItem::Sync),
        4 => Some(TyLangItem::Freeze),
        5 => Some(TyLangItem::Unpin),
        6 => Some(TyLangItem::Drop),
        7 => Some(TyLangItem::Tuple),
        8 => Some(TyLangItem::FnPtrTrait),
        9 => Some(TyLangItem::Option),
        10 => Some(TyLangItem::OptionSome),
        11 => Some(TyLangItem::OptionNone),
        12 => Some(TyLangItem::Result),
        13 => Some(TyLangItem::ResultOk),
        14 => Some(TyLangItem::ResultErr),
        15 => Some(TyLangItem::ControlFlow),
        16 => Some(TyLangItem::ControlFlowContinue),
        17 => Some(TyLangItem::ControlFlowBreak),
        18 => Some(TyLangItem::Poll),
        19 => Some(TyLangItem::PollReady),
        20 => Some(TyLangItem::PollPending),
        21 => Some(TyLangItem::Pin),
        22 => Some(TyLangItem::OwnedBox),
        23 => Some(TyLangItem::String),
        28 => Some(TyLangItem::RangeFull),
        29 => Some(TyLangItem::Range),
        30 => Some(TyLangItem::RangeFrom),
        31 => Some(TyLangItem::RangeTo),
        32 => Some(TyLangItem::RangeInclusive),
        33 => Some(TyLangItem::RangeToInclusive),
        34 => Some(TyLangItem::Try),
        35 => Some(TyLangItem::TryOutput),
        43 => Some(TyLangItem::Fn),
        45 => Some(TyLangItem::FnMut),
        47 => Some(TyLangItem::FnOnce),
        49 => Some(TyLangItem::FnOnceOutput),
        50 => Some(TyLangItem::AsyncFn),
        52 => Some(TyLangItem::AsyncFnMut),
        54 => Some(TyLangItem::AsyncFnOnce),
        56 => Some(TyLangItem::AsyncFnOnceOutput),
        59 => Some(TyLangItem::Deref),
        60 => Some(TyLangItem::DerefTarget),
        62 => Some(TyLangItem::DerefMut),
        66 => Some(TyLangItem::Index),
        67 => Some(TyLangItem::IndexOutput),
        69 => Some(TyLangItem::IndexMut),
        71 => Some(TyLangItem::Iterator),
        72 => Some(TyLangItem::IteratorItem),
        74 => Some(TyLangItem::IntoIterator),
        75 => Some(TyLangItem::IntoIteratorItem),
        78 => Some(TyLangItem::Future),
        79 => Some(TyLangItem::FutureOutput),
        81 => Some(TyLangItem::IntoFuture),
        82 => Some(TyLangItem::IntoFutureOutput),
        87 => Some(TyLangItem::Add),
        88 => Some(TyLangItem::Sub),
        89 => Some(TyLangItem::Mul),
        90 => Some(TyLangItem::Div),
        91 => Some(TyLangItem::Rem),
        92 => Some(TyLangItem::Neg),
        93 => Some(TyLangItem::Not),
        94 => Some(TyLangItem::BitXor),
        95 => Some(TyLangItem::BitAnd),
        96 => Some(TyLangItem::BitOr),
        97 => Some(TyLangItem::Shl),
        98 => Some(TyLangItem::Shr),
        _ => None,
    }
}

pub(crate) const fn assoc_lang_to_trait_and_name(item: TyLangItem) -> Option<(TyLangItem, &'static str)> {
    match item {
        TyLangItem::DerefTarget => Some((TyLangItem::Deref, "Target")),
        TyLangItem::TryOutput => Some((TyLangItem::Try, "Output")),
        TyLangItem::FutureOutput => Some((TyLangItem::Future, "Output")),
        TyLangItem::IntoFutureOutput => Some((TyLangItem::IntoFuture, "Output")),
        TyLangItem::IndexOutput => Some((TyLangItem::Index, "Output")),
        TyLangItem::IntoIteratorItem => Some((TyLangItem::IntoIterator, "Item")),
        TyLangItem::IteratorItem => Some((TyLangItem::Iterator, "Item")),
        TyLangItem::FnOnceOutput => Some((TyLangItem::FnOnce, "Output")),
        TyLangItem::AsyncFnOnceOutput => Some((TyLangItem::AsyncFnOnce, "Output")),
        TyLangItem::Add
        | TyLangItem::Sub
        | TyLangItem::Mul
        | TyLangItem::Div
        | TyLangItem::Rem
        | TyLangItem::Neg
        | TyLangItem::Not
        | TyLangItem::BitXor
        | TyLangItem::BitAnd
        | TyLangItem::BitOr
        | TyLangItem::Shl
        | TyLangItem::Shr => Some((item, "Output")),
        _ => None,
    }
}
