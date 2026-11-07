/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Lowering of `tcx.generics_of`, `tcx.predicates_of`, `tcx.explicit_super_predicates_of`,
//! `tcx.explicit_item_bounds`, and trait default `AssocTy` RHS types into `.ridx` v5
//! (`GenericParamEntry`, `WherePredEntry`, and `DefTyEntry` with `DEF_TY_FLAG_DEFAULT_ASSOC`).

use std::collections::HashSet;

use ridx_format::{
    CtorShape, GenericParamKind, IndexWriter, RawDefTy, RawGenericParam, RawWherePred, TyDefId, TyId, DEF_TY_FLAG_DEFAULT_ASSOC, GENERIC_PARAM_FLAG_HAS_DEFAULT,
    GENERIC_PARAM_FLAG_IS_SELF, GENERIC_PARAM_FLAG_SIZED, GENERIC_PARAM_FLAG_SYNTHETIC, WHERE_FLAG_CONST, WHERE_FLAG_HRTB, WHERE_FLAG_NEGATIVE,
};
use rustc_hir::def::DefKind;
use rustc_middle::ty;
use rustc_span::def_id::DefId;
use rustc_span::symbol::kw;

use super::{build_raw_generic_args, lower_region, OwnedArg, TcxTypeLowerer};

enum OwnedPred {
    TraitBound { lhs_ty: TyId, trait_def: TyDefId, args: Vec<OwnedArg>, exprs: Vec<String>, flags: u8 },
    ProjectionEq { self_ty: TyId, trait_def: TyDefId, args: Vec<OwnedArg>, exprs: Vec<String>, assoc_name: String, rhs_ty: Option<TyId>, flags: u8 },
    TypeOutlives { lhs_ty: TyId, region: u32, flags: u8 },
    RegionOutlives { lhs_region: u32, rhs_region: u32, flags: u8 },
}

struct OwnedParam {
    def_index: Option<u32>,
    name: String,
    param_idx: u16,
    kind: GenericParamKind,
    ty_or_default: Option<TyId>,
    flags: u8,
}

impl<'tcx, 'a> TcxTypeLowerer<'tcx, 'a> {
    /// Records `GenericParamEntry`, `WherePredEntry`, and trait default `AssocTy` `DefTyEntry`
    /// rows for every generic-bearing item in `items`.
    pub(crate) fn record_item_generics_and_preds(&mut self, items: &[(DefId, u32)], writer: &mut IndexWriter) {
        for &(did, def_index) in items {
            let kind = self.tcx.def_kind(did);
            if kind == DefKind::AssocTy && self.tcx.opt_item_name(did).is_some() && !self.tcx.is_impl_trait_in_trait(did) {
                let parent = self.tcx.parent(did);
                if self.tcx.def_kind(parent) == DefKind::Trait && self.tcx.defaultness(did).has_value() {
                    let default_ty = self.lower_ty(self.tcx.type_of(did).skip_binder(), writer);
                    writer.add_def_ty(RawDefTy {
                        def_index,
                        ty: default_ty,
                        parent_def_index: self.local_def_indices.get(&parent).copied(),
                        variant_idx: None,
                        ctor_shape: CtorShape::None,
                        flags: DEF_TY_FLAG_DEFAULT_ASSOC,
                    });
                }
            }

            let (params, preds) = self.collect_generics_and_preds(did, kind, writer);
            for p in &params {
                writer.add_def_generic_param(
                    def_index,
                    RawGenericParam { def_index: p.def_index, name: &p.name, param_idx: p.param_idx, kind: p.kind, ty_or_default: p.ty_or_default, flags: p.flags },
                );
            }
            for wp in &preds {
                emit_where_pred(wp, |raw| writer.add_def_where_pred(def_index, raw));
            }
        }
    }

    /// Records `GenericParamEntry` and `WherePredEntry` rows for `impl_did` onto `pending_impl_idx`.
    pub(super) fn record_impl_generics_and_preds(&mut self, impl_did: DefId, pending_impl_idx: u32, writer: &mut IndexWriter) {
        let kind = self.tcx.def_kind(impl_did);
        let (params, preds) = self.collect_generics_and_preds(impl_did, kind, writer);
        for p in &params {
            writer.add_impl_generic_param(
                pending_impl_idx,
                RawGenericParam { def_index: p.def_index, name: &p.name, param_idx: p.param_idx, kind: p.kind, ty_or_default: p.ty_or_default, flags: p.flags },
            );
        }
        for wp in &preds {
            emit_where_pred(wp, |raw| writer.add_impl_where_pred(pending_impl_idx, raw));
        }
    }

    fn collect_generics_and_preds(&mut self, did: DefId, kind: DefKind, writer: &mut IndexWriter) -> (Vec<OwnedParam>, Vec<OwnedPred>) {
        if kind == DefKind::AssocTy && self.tcx.is_impl_trait_in_trait(did) {
            return (Vec::new(), Vec::new());
        }

        let generics = self.tcx.generics_of(did);
        let mut own_non_self_type_indices: HashSet<u32> = HashSet::new();
        for p in &generics.own_params {
            if matches!(p.kind, ty::GenericParamDefKind::Type { .. }) && !(p.index == 0 && p.name == kw::SelfUpper) {
                own_non_self_type_indices.insert(p.index);
            }
        }

        let mut clauses: Vec<ty::Clause<'tcx>> = Vec::new();
        let mut seen_clauses: HashSet<ty::Clause<'tcx>> = HashSet::new();
        let mut push_clauses = |slice: &[(ty::Clause<'tcx>, rustc_span::Span)]| {
            for &(cl, _) in slice {
                if seen_clauses.insert(cl) {
                    clauses.push(cl);
                }
            }
        };

        if kind == DefKind::Trait {
            push_clauses(self.tcx.explicit_super_predicates_of(did).skip_binder());
        }
        push_clauses(self.tcx.predicates_of(did).predicates);
        if kind == DefKind::AssocTy && self.tcx.def_kind(self.tcx.parent(did)) == DefKind::Trait {
            push_clauses(self.tcx.explicit_item_bounds(did).skip_binder());
        }

        let lang = self.tcx.lang_items();
        let sized_did = lang.sized_trait();
        let meta_sized_did = lang.meta_sized_trait();
        let pointee_sized_did = lang.pointee_sized_trait();

        let mut sized_param_indices: HashSet<u32> = HashSet::new();
        let mut owned_preds = Vec::new();

        for cl in clauses {
            let binder = cl.kind();
            let base_flags = if binder.bound_vars().is_empty() { 0 } else { WHERE_FLAG_HRTB };
            match binder.skip_binder() {
                ty::ClauseKind::Trait(tp) => {
                    let trait_did = tp.trait_ref.def_id;
                    if Some(trait_did) == meta_sized_did || Some(trait_did) == pointee_sized_did {
                        continue;
                    }
                    let is_pos = matches!(tp.polarity, ty::PredicatePolarity::Positive);
                    if is_pos && kind == DefKind::Trait && trait_did == did {
                        if let ty::TyKind::Param(p) = *tp.self_ty().kind() {
                            if p.index == 0 {
                                continue;
                            }
                        }
                    }
                    if is_pos && Some(trait_did) == sized_did && base_flags == 0 {
                        if let ty::TyKind::Param(p) = *tp.self_ty().kind() {
                            sized_param_indices.insert(p.index);
                            if own_non_self_type_indices.contains(&p.index) {
                                continue;
                            }
                        }
                    }
                    let flags = base_flags | if is_pos { 0 } else { WHERE_FLAG_NEGATIVE };
                    let lhs_ty = self.lower_ty(tp.self_ty(), writer);
                    let trait_def = self.lower_def(trait_did, writer);
                    let args_slice = if tp.trait_ref.args.len() > 1 { &tp.trait_ref.args[1..] } else { &[] };
                    let (args, exprs) = self.lower_generic_args(args_slice, writer);
                    owned_preds.push(OwnedPred::TraitBound { lhs_ty, trait_def, args, exprs, flags });
                }
                ty::ClauseKind::HostEffect(hp) => {
                    let trait_did = hp.trait_ref.def_id;
                    let lhs_ty = self.lower_ty(hp.self_ty(), writer);
                    let trait_def = self.lower_def(trait_did, writer);
                    let args_slice = if hp.trait_ref.args.len() > 1 { &hp.trait_ref.args[1..] } else { &[] };
                    let (args, exprs) = self.lower_generic_args(args_slice, writer);
                    owned_preds.push(OwnedPred::TraitBound { lhs_ty, trait_def, args, exprs, flags: base_flags | WHERE_FLAG_CONST });
                }
                ty::ClauseKind::Projection(pp) => {
                    if let ty::AliasTermKind::ProjectionTy { def_id: assoc_did } = pp.projection_term.kind {
                        if let Some(sym) = self.tcx.opt_item_name(assoc_did) {
                            let self_ty = self.lower_ty(pp.projection_term.self_ty(), writer);
                            let trait_did = self.tcx.parent(assoc_did);
                            let trait_def = self.lower_def(trait_did, writer);
                            let args_slice = if pp.projection_term.args.len() > 1 { &pp.projection_term.args[1..] } else { &[] };
                            let (args, exprs) = self.lower_generic_args(args_slice, writer);
                            let rhs_ty = pp.term.as_type().map(|t| self.lower_ty(t, writer));
                            owned_preds.push(OwnedPred::ProjectionEq { self_ty, trait_def, args, exprs, assoc_name: sym.to_string(), rhs_ty, flags: base_flags });
                        }
                    }
                }
                ty::ClauseKind::TypeOutlives(ty::OutlivesPredicate(lhs, reg)) => {
                    let lhs_ty = self.lower_ty(lhs, writer);
                    owned_preds.push(OwnedPred::TypeOutlives { lhs_ty, region: lower_region(reg), flags: base_flags });
                }
                ty::ClauseKind::RegionOutlives(ty::OutlivesPredicate(r1, r2)) => {
                    owned_preds.push(OwnedPred::RegionOutlives { lhs_region: lower_region(r1), rhs_region: lower_region(r2), flags: base_flags });
                }
                _ => {}
            }
        }

        let mut owned_params = Vec::with_capacity(generics.own_params.len());
        for p in &generics.own_params {
            let def_index = self.local_def_indices.get(&p.def_id).copied();
            let param_idx = u16::try_from(p.index).unwrap_or(u16::MAX);
            match p.kind {
                ty::GenericParamDefKind::Lifetime => {
                    let flags = if p.is_anonymous_lifetime() { GENERIC_PARAM_FLAG_SYNTHETIC } else { 0 };
                    owned_params.push(OwnedParam { def_index, name: p.name.to_string(), param_idx, kind: GenericParamKind::Lifetime, ty_or_default: None, flags });
                }
                ty::GenericParamDefKind::Type { has_default, synthetic } => {
                    let is_self = p.index == 0 && p.name == kw::SelfUpper;
                    let mut flags = 0;
                    if has_default {
                        flags |= GENERIC_PARAM_FLAG_HAS_DEFAULT;
                    }
                    if synthetic {
                        flags |= GENERIC_PARAM_FLAG_SYNTHETIC;
                    }
                    if is_self {
                        flags |= GENERIC_PARAM_FLAG_IS_SELF;
                    }
                    if sized_param_indices.contains(&p.index) {
                        flags |= GENERIC_PARAM_FLAG_SIZED;
                    }
                    let ty_or_default = if has_default && !is_self { Some(self.lower_ty(self.tcx.type_of(p.def_id).skip_binder(), writer)) } else { None };
                    owned_params.push(OwnedParam { def_index, name: p.name.to_string(), param_idx, kind: GenericParamKind::Type, ty_or_default, flags });
                }
                ty::GenericParamDefKind::Const { has_default } => {
                    let flags = if has_default { GENERIC_PARAM_FLAG_HAS_DEFAULT } else { 0 };
                    let const_ty = self.lower_ty(self.tcx.type_of(p.def_id).skip_binder(), writer);
                    owned_params.push(OwnedParam { def_index, name: p.name.to_string(), param_idx, kind: GenericParamKind::Const, ty_or_default: Some(const_ty), flags });
                }
            }
        }

        (owned_params, owned_preds)
    }
}

fn emit_where_pred<F: FnMut(RawWherePred<'_>)>(wp: &OwnedPred, mut f: F) {
    match wp {
        OwnedPred::TraitBound { lhs_ty, trait_def, args, exprs, flags } => {
            let raw_args = build_raw_generic_args(args, exprs);
            f(RawWherePred::TraitBound { lhs_ty: *lhs_ty, trait_def: *trait_def, trait_args: &raw_args, flags: *flags });
        }
        OwnedPred::ProjectionEq { self_ty, trait_def, args, exprs, assoc_name, rhs_ty, flags } => {
            let raw_args = build_raw_generic_args(args, exprs);
            f(RawWherePred::ProjectionEq { self_ty: *self_ty, trait_def: *trait_def, trait_args: &raw_args, assoc_name: assoc_name.as_str(), rhs_ty: *rhs_ty, flags: *flags });
        }
        OwnedPred::TypeOutlives { lhs_ty, region, flags } => {
            f(RawWherePred::TypeOutlives { lhs_ty: *lhs_ty, region: *region, flags: *flags });
        }
        OwnedPred::RegionOutlives { lhs_region, rhs_region, flags } => {
            f(RawWherePred::RegionOutlives { lhs_region: *lhs_region, rhs_region: *rhs_region, flags: *flags });
        }
    }
}
