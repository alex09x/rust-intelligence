/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The associated type a type-relative path in a signature names (`Self::Item`, `V::Value`): outside bodies rustc
//! keeps no typeck result for it, so the trait is found from `Self` or from the type parameter's bounds.

use rustc_hir as hir;
use rustc_hir::def::{DefKind, Res};
use rustc_middle::ty::{ClauseKind, TyCtxt, TyKind};
use rustc_span::def_id::DefId;
use rustc_span::Symbol;

/// The definition of associated type `name` reached through `qself` (`Self` of a trait impl or of a trait, or a type
/// parameter), searching the bounding traits and their supertraits; `Res::Err` when there is none.
pub(crate) fn type_relative_assoc_res<'tcx>(tcx: TyCtxt<'tcx>, qself: &hir::Ty<'tcx>, name: Symbol) -> Res {
    let hir::TyKind::Path(hir::QPath::Resolved(None, path)) = qself.kind else { return Res::Err };
    let traits: Vec<DefId> = match path.res {
        Res::SelfTyAlias { alias_to, .. } if matches!(tcx.def_kind(alias_to), DefKind::Impl { of_trait: true }) => vec![tcx.impl_trait_ref(alias_to).skip_binder().def_id],
        Res::SelfTyParam { trait_ } => vec![trait_],
        Res::Def(DefKind::TyParam, param) => param_bounds(tcx, param),
        _ => return Res::Err,
    };
    traits.into_iter().find_map(|tr| trait_assoc_ty(tcx, tr, name)).map_or(Res::Err, |d| Res::Def(DefKind::AssocTy, d))
}

/// Associated type `name` of trait `tr` or of one of its supertraits.
pub(crate) fn trait_assoc_ty(tcx: TyCtxt<'_>, tr: DefId, name: Symbol) -> Option<DefId> {
    let mut traits = vec![tr];
    let mut at = 0;
    while at < traits.len() && traits.len() < 32 {
        let t = traits[at];
        at += 1;
        // Synthetic associated types (`impl Trait` in a trait method's return) have no name.
        if let Some(d) = tcx.associated_item_def_ids(t).iter().copied().find(|&d| tcx.def_kind(d) == DefKind::AssocTy && tcx.opt_item_name(d) == Some(name)) {
            return Some(d);
        }
        for (clause, _) in tcx.explicit_super_predicates_of(t).skip_binder() {
            if let ClauseKind::Trait(tp) = clause.kind().skip_binder() {
                if !traits.contains(&tp.trait_ref.def_id) {
                    traits.push(tp.trait_ref.def_id);
                }
            }
        }
    }
    None
}

/// The traits bounding type parameter `param` in the predicates of the item that declares it and its parents.
fn param_bounds(tcx: TyCtxt<'_>, param: DefId) -> Vec<DefId> {
    let Some(param_name) = tcx.opt_item_name(param) else { return Vec::new() };
    let mut out = Vec::new();
    let mut cur = Some(tcx.predicates_of(tcx.parent(param)));
    while let Some(preds) = cur {
        for &(clause, _) in preds.predicates {
            if let ClauseKind::Trait(tp) = clause.kind().skip_binder() {
                if matches!(tp.self_ty().kind(), TyKind::Param(p) if p.name == param_name) && !out.contains(&tp.trait_ref.def_id) {
                    out.push(tp.trait_ref.def_id);
                }
            }
        }
        cur = preds.parent.map(|p| tcx.predicates_of(p));
    }
    out
}
