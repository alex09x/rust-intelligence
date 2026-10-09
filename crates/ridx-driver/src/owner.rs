/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Canonical owner type and generic trait bound resolution for `.ridx` format v3.

use rustc_middle::ty::TyCtxt;

/// Resolves the canonical owner path string for an inferred expression or pattern type.
///
/// Supports:
/// - Concrete ADTs (structs, enums, unions) -> canonical def path string (e.g. `bevy_ecs::world::World`)
/// - Dynamic trait objects (`dyn Trait`) -> principal trait path string
/// - Generic type parameters (`T: Trait`) -> primary bounded trait path string via `predicates_of` / `param_env`
/// - Primitives and builtins (`str`, `slice`, `bool`, integer/float primitives)
pub fn resolve_owner_from_ty<'tcx>(
    tcx: TyCtxt<'tcx>,
    ty: rustc_middle::ty::Ty<'tcx>,
    param_env: Option<rustc_middle::ty::ParamEnv<'tcx>>,
) -> Option<String> {
    let peeled = ty.peel_refs();
    if let Some(adt_def) = peeled.ty_adt_def() {
        let did = adt_def.did();
        let path = rustc_middle::ty::print::with_no_visible_paths!(
            rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(did))
        );
        return Some(path);
    }
    match peeled.kind() {
        rustc_middle::ty::TyKind::Dynamic(predicates, ..) => {
            if let Some(principal) = predicates.principal() {
                let did = principal.def_id();
                let path = rustc_middle::ty::print::with_no_visible_paths!(
                    rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(did))
                );
                return Some(path);
            }
        }
        rustc_middle::ty::TyKind::Param(param_ty) => {
            if let Some(env) = param_env {
                return resolve_owner_from_param(tcx, *param_ty, env);
            }
        }
        rustc_middle::ty::TyKind::Str => {
            return Some("str".to_string());
        }
        rustc_middle::ty::TyKind::Slice(_) => {
            return Some("slice".to_string());
        }
        rustc_middle::ty::TyKind::Bool
        | rustc_middle::ty::TyKind::Char
        | rustc_middle::ty::TyKind::Int(_)
        | rustc_middle::ty::TyKind::Uint(_)
        | rustc_middle::ty::TyKind::Float(_) => {
            return Some(peeled.to_string());
        }
        _ => {}
    }
    None
}

/// Resolves the primary owner trait for an unconstrained generic type parameter `T` from `param_env` caller bounds.
///
/// Iterates trait predicate clauses bounding `param_ty`, filtering out `Sized` and auto-traits (`Send`, `Sync`, `Unpin`).
/// Prioritizes workspace/dependency traits over standard sysroot crates and traits declaring associated items/methods.
pub fn resolve_owner_from_param<'tcx>(
    tcx: TyCtxt<'tcx>,
    param_ty: rustc_middle::ty::ParamTy,
    param_env: rustc_middle::ty::ParamEnv<'tcx>,
) -> Option<String> {
    let mut candidate_traits = Vec::new();

    for clause in param_env.caller_bounds() {
        if let rustc_middle::ty::ClauseKind::Trait(trait_pred) = clause.kind().skip_binder() {
            let self_ty = trait_pred.self_ty();
            if let rustc_middle::ty::TyKind::Param(p) = self_ty.kind() {
                if p.index == param_ty.index {
                    let trait_did = trait_pred.trait_ref.def_id;
                    if tcx.lang_items().sized_trait() == Some(trait_did) || tcx.trait_is_auto(trait_did) {
                        continue;
                    }
                    let trait_path = rustc_middle::ty::print::with_no_visible_paths!(
                        rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(trait_did))
                    );
                    if trait_path.starts_with("core::marker::") {
                        continue;
                    }
                    candidate_traits.push((trait_did, trait_path));
                }
            }
        }
    }

    if candidate_traits.is_empty() {
        return None;
    }

    // Prioritize non-sysroot domain traits (e.g. `serde::de::Visitor` over `core::fmt::Debug`)
    // and traits with declared methods/items.
    candidate_traits.sort_by_key(|&(did, ref path)| {
        let is_sysroot = path.starts_with("core::")
            || path.starts_with("alloc::")
            || path.starts_with("std::");
        let has_items = !tcx.associated_item_def_ids(did).is_empty();
        (is_sysroot, !has_items)
    });

    Some(candidate_traits.remove(0).1)
}
