/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Full [`ridx_syntax::ty_infer::TyDefDb`] implementation on [`super::v5_db::RidxV5ItemDb`]
//! backed by `.ridx` `v5` `FnSigEntry`, `DefTyEntry`, `FieldEntry`, `ImplEntry`, `WherePredEntry`,
//! and pre-indexed [`super::v5_ty_tables::V5TyTables`].

use ridx_format::{CtorShape, DefKind, TyKindView, WherePredKind};
use ridx_syntax::name_res::{CrateId, GlobalDefId, ImplTargetKey};
use ridx_syntax::ty_infer::uf_table::InlineTyBuf;
use ridx_syntax::ty_infer::{CtorShapeKind, InferTyArena, InferTyId, InferTyKind, MethodCand, TraitAssocFnMeta, TyDefDb, TyLangItem, UnionFindTable, WhereObligation};

use super::v5_db::RidxV5ItemDb;
use super::v5_ty_defaults::assoc_lang_to_trait_and_name;
use super::v5_ty_lower::{lower_generic_args, lower_v5_ty, ParamRemap};
use super::v5_ty_tables::fnv1a_str;

impl<'w> TyDefDb for RidxV5ItemDb<'w> {
    fn fn_sig(&self, raw_def: GlobalDefId, arena: &mut InferTyArena, inputs_out: &mut Vec<InferTyId>) -> Option<(u16, InferTyId)> {
        let def = self.canonical_def(raw_def);
        let r_idx = def.krate.0 as usize;
        let reader = self.ws.readers().get(r_idx)?;
        let sig = reader.fn_sig_for_def(def.index)?;
        let gen_owner = reader.def_ty_for_def(def.index).and_then(|dt| dt.parent_def_index()).unwrap_or(def.index);
        let remap = if gen_owner != def.index { ParamRemap::for_def(self, r_idx, gen_owner, false, arena) } else { ParamRemap::for_def(self, r_idx, def.index, false, arena) };
        let va = reader.ty_arena();
        inputs_out.clear();
        for inp in sig.inputs() {
            inputs_out.push(lower_v5_ty(self, r_idx, va, inp, &remap, arena));
        }
        let out = lower_v5_ty(self, r_idx, va, sig.output(), &remap, arena);
        Some((remap.non_lt_count, out))
    }

    fn def_ty(&self, raw_def: GlobalDefId, arena: &mut InferTyArena) -> Option<(u16, CtorShapeKind, InferTyId)> {
        let def = self.canonical_def(raw_def);
        let r_idx = def.krate.0 as usize;
        let reader = self.ws.readers().get(r_idx)?;
        let dt = reader.def_ty_for_def(def.index)?;
        let gen_owner = dt.parent_def_index().unwrap_or(def.index);
        let remap = ParamRemap::for_def(self, r_idx, gen_owner, false, arena);
        let shape = match dt.ctor_shape() {
            CtorShape::None => CtorShapeKind::None,
            CtorShape::Unit => CtorShapeKind::Unit,
            CtorShape::Tuple => CtorShapeKind::Tuple,
            CtorShape::Record => CtorShapeKind::Record,
        };
        let ty = lower_v5_ty(self, r_idx, reader.ty_arena(), dt.ty(), &remap, arena);
        Some((remap.non_lt_count, shape, ty))
    }

    fn ctor_tuple_fields(&self, raw_def: GlobalDefId, arena: &mut InferTyArena, fields_out: &mut Vec<InferTyId>) -> Option<u16> {
        let def = self.canonical_def(raw_def);
        let r_idx = def.krate.0 as usize;
        let reader = self.ws.readers().get(r_idx)?;
        let dt = reader.def_ty_for_def(def.index)?;
        if dt.ctor_shape() != CtorShape::Tuple {
            return None;
        }
        if let Some((gc, _)) = self.fn_sig(def, arena, fields_out) {
            return Some(gc);
        }
        let gen_owner = dt.parent_def_index().unwrap_or(def.index);
        let remap = ParamRemap::for_def(self, r_idx, gen_owner, false, arena);
        let va = reader.ty_arena();
        fields_out.clear();
        for fv in reader.fields_for_owner(def.index) {
            fields_out.push(lower_v5_ty(self, r_idx, va, fv.ty, &remap, arena));
        }
        Some(remap.non_lt_count)
    }

    fn field_ty(&self, owner_def: GlobalDefId, name: &str, arena: &mut InferTyArena) -> Option<(u16, InferTyId)> {
        self.field_ty_and_def(owner_def, name, arena).map(|(gc, ty, _)| (gc, ty))
    }

    fn field_ty_and_def(&self, raw_owner: GlobalDefId, name: &str, arena: &mut InferTyArena) -> Option<(u16, InferTyId, GlobalDefId)> {
        let owner = self.canonical_def(raw_owner);
        let r_idx = owner.krate.0 as usize;
        let reader = self.ws.readers().get(r_idx)?;
        let fv = reader.find_field(owner.index, name)?;
        let gen_owner = reader.def_ty_for_def(owner.index).and_then(|dt| dt.parent_def_index()).unwrap_or(owner.index);
        let remap = ParamRemap::for_def(self, r_idx, gen_owner, false, arena);
        let fty = lower_v5_ty(self, r_idx, reader.ty_arena(), fv.ty, &remap, arena);
        let fgid = fv.def_index.map_or(owner, |di| GlobalDefId::new(CrateId(r_idx as u32), di));
        Some((remap.non_lt_count, fty, fgid))
    }

    fn lang_item(&self, item: TyLangItem) -> Option<GlobalDefId> {
        self.ty_tables.lang_to_gid.get(&(item as u16)).copied()
    }

    fn lang_item_of_def(&self, def: GlobalDefId) -> Option<TyLangItem> {
        self.ty_tables.gid_to_lang.get(&self.canonical_def(def)).copied()
    }

    fn cstr_def(&self) -> Option<GlobalDefId> {
        self.ty_tables.cstr_gid
    }

    fn project_assoc_ty(&self, assoc_item: TyLangItem, self_ty: InferTyId, trait_args: &[InferTyId], arena: &mut InferTyArena, vars: &mut UnionFindTable) -> Option<InferTyId> {
        let (tr_lang, aname) = assoc_lang_to_trait_and_name(assoc_item)?;
        let tr_gid = self.lang_item(tr_lang)?;
        self.project_by_trait_and_name(tr_gid, aname, self_ty, trait_args, arena, vars)
    }

    fn project_trait_assoc(
        &self,
        trait_def: GlobalDefId,
        assoc_def: GlobalDefId,
        self_ty: InferTyId,
        trait_args: &[InferTyId],
        arena: &mut InferTyArena,
        vars: &mut UnionFindTable,
    ) -> Option<InferTyId> {
        let tgid = self.canonical_def(trait_def);
        let agid = self.canonical_def(assoc_def);
        if !tgid.is_some() && !agid.is_some() {
            let cur = vars.shallow_resolve(arena, self_ty);
            if let InferTyKind::Slice(elem) = arena.kind(cur) {
                let re = vars.shallow_resolve(arena, elem);
                if trait_args.first().is_some_and(|&ta| vars.shallow_resolve(arena, ta) == InferTyId::STR_REF)
                    || (!arena.flags(re).has_param()
                        && (re == InferTyId::STR_REF
                            || re == InferTyId::STR
                            || matches!(arena.kind(re), InferTyKind::Adt { def, .. } if self.lang_item_of_def(def) == Some(TyLangItem::String))))
                {
                    return self.v5_std_macro_ty(ridx_syntax::ty_infer::StdMacroTy::String, arena);
                }
                if let InferTyKind::Ref { inner, .. } = arena.kind(re) {
                    if let InferTyKind::Slice(se) = arena.kind(vars.shallow_resolve(arena, inner)) {
                        return self.v5_std_macro_ty(ridx_syntax::ty_infer::StdMacroTy::Vec(se), arena);
                    }
                }
            }
            return None;
        }
        let aname = self.ty_tables.assoc_ty_names.get(&agid)?.as_str(self.ws);
        self.project_by_trait_and_name(tgid, aname, self_ty, trait_args, arena, vars)
    }

    fn param_fn_bound(&self, raw_owner: GlobalDefId, param_idx: u16, arena: &mut InferTyArena, inputs_out: &mut Vec<InferTyId>) -> Option<(TyLangItem, InferTyId)> {
        let owner = self.canonical_def(raw_owner);
        let r_idx = owner.krate.0 as usize;
        let reader = self.ws.readers().get(r_idx)?;
        let remap = ParamRemap::for_def(self, r_idx, owner.index, false, arena);
        let va = reader.ty_arena();

        let mut fn_tr: Option<(TyLangItem, GlobalDefId, InlineTyBuf)> = None;
        let mut out_rhs: Option<InferTyId> = None;
        for wp in self.iter_owner_where_preds(r_idx, owner.index) {
            match wp.kind() {
                WherePredKind::TraitBound if !wp.is_negative() => {
                    let TyKindView::Param { index, .. } = va.kind(wp.lhs_ty()) else {
                        continue;
                    };
                    if remap.dense_idx_of(index, arena) != Some(param_idx) {
                        continue;
                    }
                    let Some(td) = wp.trait_def() else {
                        continue;
                    };
                    let tgid = self.ty_def_gid(r_idx, td);
                    let Some(tl) = self.lang_item_of_def(tgid).filter(|l| l.is_fn_trait()) else {
                        continue;
                    };
                    let mut targs = InlineTyBuf::new();
                    lower_generic_args(self, r_idx, va, wp.trait_args(), &remap, arena, &mut targs, 1);
                    fn_tr = Some((tl, tgid, targs));
                }
                WherePredKind::ProjectionEq => {
                    let TyKindView::Param { index, .. } = va.kind(wp.lhs_ty()) else {
                        continue;
                    };
                    if remap.dense_idx_of(index, arena) == Some(param_idx) && wp.assoc_name() == Some("Output") {
                        if let Some(rhs) = wp.rhs_ty() {
                            out_rhs = Some(lower_v5_ty(self, r_idx, va, rhs, &remap, arena));
                        }
                    }
                }
                _ => {}
            }
        }
        let (tl, _tgid, targs) = fn_tr?;
        inputs_out.clear();
        if let Some(&tup) = targs.as_slice().first() {
            match arena.kind(tup) {
                InferTyKind::Tuple(r) => inputs_out.extend_from_slice(arena.slice(r)),
                _ if tup == InferTyId::UNIT => {}
                _ => inputs_out.push(tup),
            }
        }
        Some((tl, out_rhs.unwrap_or(InferTyId::UNIT)))
    }

    fn inherent_method_cands(&self, target: ImplTargetKey, method_name: &str, _arena: &mut InferTyArena, out: &mut Vec<MethodCand>) {
        let key = (norm_target_key(self, target), fnv1a_str(method_name));
        let list = self.ty_tables.inherent_slice(&key);
        for info in list.iter().filter(|i| i.name.as_str(self.ws) == method_name) {
            let mdef = GlobalDefId::new(CrateId(info.r_idx), info.method_didx);
            if !out.iter().any(|c| c.method_def == mdef) {
                out.push(MethodCand {
                    method_def: mdef,
                    trait_def: GlobalDefId::NONE,
                    bound_owner_def: mdef,
                    impl_owner: GlobalDefId::NONE,
                    container_generics: info.impl_gc,
                    method_generics: info.method_gc,
                    has_fixed_trait_subst: false,
                    fixed_self_ty: InferTyId::NONE,
                    fixed_trait_args: ridx_syntax::ty_infer::TySliceRange::EMPTY,
                    is_blanket_impl: false,
                    cst_fn_node: u32::MAX,
                });
            }
        }
    }

    fn trait_impl_method_cands(
        &self,
        raw_target: ImplTargetKey,
        raw_trait: GlobalDefId,
        method_name: &str,
        step_ty: InferTyId,
        arena: &mut InferTyArena,
        vars: &mut UnionFindTable,
        out: &mut Vec<MethodCand>,
    ) {
        let trait_def = self.canonical_def(raw_trait);
        let target = norm_target_key(self, raw_target);
        let Some(&tm_info) = self.ty_tables.trait_assoc_fns.get(&(trait_def, fnv1a_str(method_name))).filter(|i| i.has_self && i.name.as_str(self.ws) == method_name) else {
            return;
        };
        let impls = if target == ImplTargetKey::Param { self.ty_tables.blanket_impls(&trait_def) } else { self.ty_tables.concrete_impls(&(trait_def, target)) };
        if impls.is_empty() {
            return;
        }
        let trait_mdef = GlobalDefId::new(trait_def.krate, tm_info.method_didx);

        for imp in impls {
            let imp_owner = GlobalDefId::new(CrateId(imp.r_idx), imp.impl_idx);
            if imp.is_blanket && out.iter().any(|c| c.impl_owner == imp_owner) {
                continue;
            }
            let Some((raw_self, raw_targs)) = self.probe_impl_compat(imp, step_ty, arena, vars) else { continue };
            let r_idx = imp.r_idx as usize;
            let reader = &self.ws.readers()[r_idx];
            let Some(iv) = reader.impl_view(imp.impl_idx) else { continue };
            let over_didx = iv.assoc_def_indices().find(|&adidx| {
                reader
                    .definitions()
                    .get(adidx as usize)
                    .is_some_and(|ad| DefKind::from(ad.kind) == DefKind::AssocFn && reader.get_str(ad.name_offset()).trim_start_matches("r#") == method_name)
            });
            if let Some(mdidx) = over_didx {
                let mdef = GlobalDefId::new(CrateId(imp.r_idx), mdidx);
                if !out.iter().any(|c| c.method_def == mdef && !c.has_fixed_trait_subst) {
                    let mgc = super::v5_ty_tables::count_def_non_lt_non_self(reader, mdidx);
                    out.push(MethodCand {
                        method_def: mdef,
                        trait_def,
                        bound_owner_def: mdef,
                        impl_owner: imp_owner,
                        container_generics: imp.impl_gc,
                        method_generics: mgc,
                        has_fixed_trait_subst: false,
                        fixed_self_ty: InferTyId::NONE,
                        fixed_trait_args: ridx_syntax::ty_infer::TySliceRange::EMPTY,
                        is_blanket_impl: imp.is_blanket,
                        cst_fn_node: u32::MAX,
                    });
                }
            } else {
                let mut padded = InlineTyBuf::new();
                for i in 0..tm_info.trait_gc as usize {
                    padded.push(raw_targs.as_slice().get(i).copied().unwrap_or(InferTyId::UNKNOWN));
                }
                let args_r = arena.intern_slice(padded.as_slice());
                out.push(MethodCand {
                    method_def: trait_mdef,
                    trait_def,
                    bound_owner_def: trait_mdef,
                    impl_owner: imp_owner,
                    container_generics: imp.impl_gc,
                    method_generics: tm_info.method_gc,
                    has_fixed_trait_subst: true,
                    fixed_self_ty: raw_self,
                    fixed_trait_args: args_r,
                    is_blanket_impl: imp.is_blanket,
                    cst_fn_node: u32::MAX,
                });
            }
        }
    }

    fn trait_decl_method_cand(&self, raw_trait: GlobalDefId, trait_args: &[InferTyId], self_ty: InferTyId, method_name: &str, arena: &mut InferTyArena) -> Option<MethodCand> {
        let mut trait_def = self.canonical_def(raw_trait);
        let nh = fnv1a_str(method_name);
        let tm = match self.ty_tables.trait_assoc_fns.get(&(trait_def, nh)).filter(|i| i.has_self && i.name.as_str(self.ws) == method_name) {
            Some(tm) => tm,
            None => {
                let supers = self.trait_supertraits.get(&trait_def)?;
                let (cs, hit) = supers.iter().find_map(|&s| {
                    let cs = self.canonical_def(s);
                    self.ty_tables.trait_assoc_fns.get(&(cs, nh)).filter(|i| i.has_self && i.name.as_str(self.ws) == method_name).map(|h| (cs, h))
                })?;
                trait_def = cs;
                hit
            }
        };
        let mut padded = InlineTyBuf::new();
        for i in 0..tm.trait_gc as usize {
            padded.push(trait_args.get(i).copied().unwrap_or(InferTyId::NONE));
        }
        let args_r = arena.intern_slice(padded.as_slice());
        let mdef = GlobalDefId::new(trait_def.krate, tm.method_didx);
        Some(MethodCand {
            method_def: mdef,
            trait_def,
            bound_owner_def: mdef,
            impl_owner: GlobalDefId::NONE,
            container_generics: 0,
            method_generics: tm.method_gc,
            has_fixed_trait_subst: true,
            fixed_self_ty: self_ty,
            fixed_trait_args: args_r,
            is_blanket_impl: false,
            cst_fn_node: u32::MAX,
        })
    }

    fn trait_supertraits(&self, raw_trait: GlobalDefId, out: &mut Vec<GlobalDefId>) {
        let trait_def = self.canonical_def(raw_trait);
        if let Some(supers) = self.trait_supertraits.get(&trait_def) {
            for &s in supers {
                let cs = self.canonical_def(s);
                if cs.is_some() && !out.contains(&cs) {
                    out.push(cs);
                }
            }
        }
    }

    fn param_trait_bounds(&self, raw_owner: GlobalDefId, param_idx: u16, out: &mut Vec<GlobalDefId>) {
        self.v5_param_trait_bounds(raw_owner, param_idx, out);
    }
    fn assoc_ty_parent_trait(&self, assoc_def: GlobalDefId) -> Option<GlobalDefId> {
        self.v5_assoc_ty_parent_trait(assoc_def)
    }
    fn assoc_ty_trait_bounds(&self, assoc_def: GlobalDefId, out: &mut Vec<GlobalDefId>) {
        self.v5_assoc_ty_trait_bounds(assoc_def, out);
    }
    fn check_blanket_method_bounds(&self, cand: &MethodCand, subst: &[InferTyId], arena: &mut InferTyArena, vars: &mut UnionFindTable) -> bool {
        self.v5_check_blanket_method_bounds(cand, subst, arena, vars)
    }
    fn trait_of_assoc_fn(&self, assoc_fn: GlobalDefId) -> Option<TraitAssocFnMeta> {
        self.v5_trait_of_assoc_fn(assoc_fn)
    }
    fn refine_inherent_assoc_fn(&self, assoc_fn: GlobalDefId, qself_ty: InferTyId, arena: &mut InferTyArena, vars: &mut UnionFindTable) -> GlobalDefId {
        self.v5_refine_inherent_assoc_fn(assoc_fn, qself_ty, arena, vars)
    }
    fn trait_assoc_fn_by_name(&self, trait_def: GlobalDefId, fn_name: &str) -> Option<TraitAssocFnMeta> {
        self.v5_trait_assoc_fn_by_name(trait_def, fn_name)
    }
    fn owner_where_obligations(&self, owner_def: GlobalDefId, arena: &mut InferTyArena, out: &mut Vec<WhereObligation>) {
        self.v5_owner_where_obligations(owner_def, arena, out);
    }
    fn solve_trait_obligation(&self, trait_def: GlobalDefId, self_ty: InferTyId, trait_args: &[InferTyId], arena: &mut InferTyArena, vars: &mut UnionFindTable) -> bool {
        self.v5_solve_trait_obligation(trait_def, self_ty, trait_args, arena, vars)
    }
    fn project_param_assoc(&self, owner: GlobalDefId, pidx: u16, tr: GlobalDefId, assoc: GlobalDefId, targs: &[InferTyId], arena: &mut InferTyArena) -> Option<InferTyId> {
        self.v5_project_param_assoc(owner, pidx, tr, assoc, targs, arena)
    }
    fn generic_param_default_ty(&self, def: GlobalDefId, param_idx: u16, arena: &mut InferTyArena) -> Option<InferTyId> {
        self.v5_generic_param_default_ty(def, param_idx, arena)
    }
    fn std_macro_ty(&self, kind: ridx_syntax::ty_infer::StdMacroTy, arena: &mut InferTyArena) -> Option<InferTyId> {
        self.v5_std_macro_ty(kind, arena)
    }
}

fn norm_target_key(db: &RidxV5ItemDb<'_>, key: ImplTargetKey) -> ImplTargetKey {
    match key {
        ImplTargetKey::Adt(d) => ImplTargetKey::Adt(db.canonical_def(d)),
        ImplTargetKey::Dyn(d) => ImplTargetKey::Dyn(db.canonical_def(d)),
        other => other,
    }
}
