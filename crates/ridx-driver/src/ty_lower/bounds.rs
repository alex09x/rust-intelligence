/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Lowering of `dyn Trait`, `impl Trait` (opaque), projection `<T as Trait>::Assoc`,
//! and inherent/free alias types into `TyArena`.

use ridx_format::{IndexWriter, RawGenericArg, RawTy, RawTyBound, TyDefId, TyId};
use rustc_middle::ty;
use rustc_span::def_id::DefId;

use super::{build_raw_generic_args, lower_region, OwnedArg, TcxTypeLowerer};

struct OwnedBound {
    trait_did: DefId,
    trait_def: TyDefId,
    args: Vec<OwnedArg>,
    exprs: Vec<String>,
    assocs: Vec<(String, TyId)>,
    is_maybe: bool,
}

impl<'tcx, 'a> TcxTypeLowerer<'tcx, 'a> {
    pub(super) fn lower_dynamic_ty(&mut self, preds: &'tcx ty::List<ty::PolyExistentialPredicate<'tcx>>, reg: ty::Region<'tcx>, writer: &mut IndexWriter) -> TyId {
        let mut owned_bounds: Vec<OwnedBound> = Vec::new();
        let mut pending_proj: Vec<(DefId, String, TyId)> = Vec::new();

        for pred in preds.iter() {
            match pred.skip_binder() {
                ty::ExistentialPredicate::Trait(tr) => {
                    let trait_def = self.lower_def(tr.def_id, writer);
                    let (args, exprs) = self.lower_generic_args(tr.args, writer);
                    owned_bounds.push(OwnedBound { trait_did: tr.def_id, trait_def, args, exprs, assocs: Vec::new(), is_maybe: false });
                }
                ty::ExistentialPredicate::AutoTrait(auto_did) => {
                    let trait_def = self.lower_def(auto_did, writer);
                    owned_bounds.push(OwnedBound { trait_did: auto_did, trait_def, args: Vec::new(), exprs: Vec::new(), assocs: Vec::new(), is_maybe: false });
                }
                ty::ExistentialPredicate::Projection(pr) => {
                    if let (Some(rhs_ty), Some(sym)) = (pr.term.as_type(), self.tcx.opt_item_name(pr.def_id)) {
                        let lowered_rhs = self.lower_ty(rhs_ty, writer);
                        let parent_trait = self.tcx.parent(pr.def_id);
                        pending_proj.push((parent_trait, sym.to_string(), lowered_rhs));
                    }
                }
            }
        }

        attach_projections(&mut owned_bounds, pending_proj);
        intern_owned_bounds(&owned_bounds, Some(lower_region(reg)), writer)
    }

    pub(super) fn lower_alias_ty(&mut self, alias_ty: ty::AliasTy<'tcx>, writer: &mut IndexWriter) -> TyId {
        match alias_ty.kind {
            ty::AliasTyKind::Projection { def_id } => {
                if let Some(name) = self.tcx.opt_item_name(def_id) {
                    let trait_ref = alias_ty.trait_ref(self.tcx);
                    let trait_def = self.lower_def(trait_ref.def_id, writer);
                    let self_ty = self.lower_ty(alias_ty.self_ty(), writer);
                    let trait_args_slice = if trait_ref.args.len() > 1 { &trait_ref.args[1..] } else { &[] };
                    let (owned_args, exprs) = self.lower_generic_args(trait_args_slice, writer);
                    let raw_args = build_raw_generic_args(&owned_args, &exprs);
                    writer.intern_ty(RawTy::Assoc { trait_def, name: name.as_str(), self_ty, trait_args: &raw_args })
                } else {
                    // RPITIT (`fn foo(&self) -> impl Trait` in trait) produces an anonymous `AnonAssocTy`
                    // with no `item_name`; lower its instantiated `explicit_item_bounds` as `ImplTrait`.
                    self.lower_item_bounds_as_opaque(def_id, alias_ty.args, writer)
                }
            }
            ty::AliasTyKind::Opaque { def_id } => self.lower_item_bounds_as_opaque(def_id, alias_ty.args, writer),
            ty::AliasTyKind::Inherent { def_id } | ty::AliasTyKind::Free { def_id } => {
                if self.alias_stack.contains(&def_id) {
                    return writer.intern_ty(RawTy::Unknown);
                }
                self.alias_stack.push(def_id);
                let expanded = self.tcx.type_of(def_id).instantiate(self.tcx, alias_ty.args).skip_normalization();
                let res = self.lower_ty(expanded, writer);
                self.alias_stack.pop();
                res
            }
        }
    }

    fn lower_item_bounds_as_opaque(&mut self, def_id: DefId, args: ty::GenericArgsRef<'tcx>, writer: &mut IndexWriter) -> TyId {
        if self.alias_stack.contains(&def_id) {
            return writer.intern_ty(RawTy::Unknown);
        }
        self.alias_stack.push(def_id);
        let sized_did = self.tcx.lang_items().sized_trait();
        let mut owned_bounds: Vec<OwnedBound> = Vec::new();
        let mut pending_proj: Vec<(DefId, String, TyId)> = Vec::new();

        let clauses: Vec<ty::Clause<'tcx>> = self.tcx.explicit_item_bounds(def_id).iter_instantiated_copied(self.tcx, args).map(|u| u.skip_normalization().0).collect();

        for clause in clauses {
            match clause.kind().skip_binder() {
                ty::ClauseKind::Trait(tp) => {
                    let trait_did = tp.trait_ref.def_id;
                    let is_maybe = !matches!(tp.polarity, ty::PredicatePolarity::Positive);
                    if !is_maybe && sized_did == Some(trait_did) {
                        continue;
                    }
                    if owned_bounds.iter().any(|b| b.trait_did == trait_did) {
                        continue;
                    }
                    let trait_def = self.lower_def(trait_did, writer);
                    let args_slice = if tp.trait_ref.args.len() > 1 { &tp.trait_ref.args[1..] } else { &[] };
                    let (b_args, exprs) = self.lower_generic_args(args_slice, writer);
                    owned_bounds.push(OwnedBound { trait_did, trait_def, args: b_args, exprs, assocs: Vec::new(), is_maybe });
                }
                ty::ClauseKind::Projection(pp) => {
                    if let ty::AliasTermKind::ProjectionTy { def_id: assoc_did } = pp.projection_term.kind {
                        if let (Some(rhs_ty), Some(sym)) = (pp.term.as_type(), self.tcx.opt_item_name(assoc_did)) {
                            let lowered_rhs = self.lower_ty(rhs_ty, writer);
                            let parent_trait = self.tcx.parent(assoc_did);
                            pending_proj.push((parent_trait, sym.to_string(), lowered_rhs));
                        }
                    }
                }
                _ => {}
            }
        }

        self.alias_stack.pop();
        attach_projections(&mut owned_bounds, pending_proj);
        intern_owned_bounds(&owned_bounds, None, writer)
    }
}

fn attach_projections(owned_bounds: &mut [OwnedBound], pending_proj: Vec<(DefId, String, TyId)>) {
    for (parent_trait, name, rhs) in pending_proj {
        if let Some(target) = owned_bounds.iter_mut().find(|b| b.trait_did == parent_trait) {
            if !target.assocs.iter().any(|(n, _)| n == &name) {
                target.assocs.push((name, rhs));
            }
        } else if let Some(first) = owned_bounds.first_mut() {
            if !first.assocs.iter().any(|(n, _)| n == &name) {
                first.assocs.push((name, rhs));
            }
        }
    }
}

fn intern_owned_bounds(owned_bounds: &[OwnedBound], dyn_lifetime: Option<u32>, writer: &mut IndexWriter) -> TyId {
    let raw_arg_vecs: Vec<Vec<RawGenericArg<'_>>> = owned_bounds.iter().map(|b| build_raw_generic_args(&b.args, &b.exprs)).collect();
    let assoc_vecs: Vec<Vec<(&str, TyId)>> = owned_bounds.iter().map(|b| b.assocs.iter().map(|(n, t)| (n.as_str(), *t)).collect()).collect();
    let raw_bounds: Vec<RawTyBound<'_>> =
        owned_bounds.iter().enumerate().map(|(i, b)| RawTyBound { trait_def: b.trait_def, args: &raw_arg_vecs[i], assocs: &assoc_vecs[i], is_maybe: b.is_maybe }).collect();
    match dyn_lifetime {
        Some(lifetime) => writer.intern_ty(RawTy::Dyn { bounds: &raw_bounds, lifetime }),
        None => writer.intern_ty(RawTy::ImplTrait { bounds: &raw_bounds }),
    }
}
