/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::*;

pub(crate) fn extract_crate_items<'tcx>(tcx: TyCtxt<'tcx>, visitor: &mut HirVisitor<'tcx>, package_root: Option<&Path>) {
    let mut items: Vec<rustc_hir::def_id::LocalDefId> = tcx.hir_crate_items(()).definitions().collect();
    let mut seen_items: std::collections::HashSet<rustc_hir::def_id::LocalDefId> = items.iter().copied().collect();
    for &def_id in &items.clone() {
        let kind = tcx.def_kind(def_id);
        if matches!(kind, hir::def::DefKind::Struct | hir::def::DefKind::Enum | hir::def::DefKind::Union) {
            let adt = tcx.adt_def(def_id);
            for variant in adt.variants() {
                if kind == hir::def::DefKind::Enum {
                    if let Some(v_did) = variant.def_id.as_local() {
                        if seen_items.insert(v_did) {
                            items.push(v_did);
                        }
                    }
                }
                for field in &variant.fields {
                    if !field.name.as_str().bytes().all(|b| b.is_ascii_digit()) {
                        if let Some(f_did) = field.did.as_local() {
                            if seen_items.insert(f_did) {
                                items.push(f_did);
                            }
                        }
                    }
                }
            }
        }
        if matches!(
            kind,
            hir::def::DefKind::Fn
                | hir::def::DefKind::AssocFn
                | hir::def::DefKind::Struct
                | hir::def::DefKind::Enum
                | hir::def::DefKind::Union
                | hir::def::DefKind::Trait
                | hir::def::DefKind::TraitAlias
                | hir::def::DefKind::Impl { .. }
                | hir::def::DefKind::TyAlias
                | hir::def::DefKind::AssocTy
        ) {
            let generics = tcx.generics_of(def_id.to_def_id());
            for param in &generics.own_params {
                if let rustc_middle::ty::GenericParamDefKind::Type { .. } = param.kind {
                    if param.name != rustc_span::symbol::kw::SelfUpper {
                        if let Some(local_did) = param.def_id.as_local() {
                            if tcx.def_kind(local_did) == hir::def::DefKind::TyParam && seen_items.insert(local_did) {
                                items.push(local_did);
                            }
                        }
                    }
                }
            }
        }
    }

    // Extract all crate item definitions
    let mut local_def_indices: std::collections::HashMap<DefId, u32> = std::collections::HashMap::with_capacity(items.len());
    let mut callables: Vec<(DefId, u32)> = Vec::new();
    let mut impl_dids: Vec<DefId> = Vec::new();
    let mut adts: Vec<(DefId, u32)> = Vec::new();
    let mut def_tys: Vec<(DefId, u32)> = Vec::new();
    for def_id in items {
        let kind = tcx.def_kind(def_id);
        let did = def_id.to_def_id();
        if matches!(kind, hir::def::DefKind::Impl { .. }) {
            impl_dids.push(did);
        }
        let item_span = tcx.def_ident_span(def_id).unwrap_or_else(|| tcx.def_span(def_id));
        let Some((file, s_line, s_col, e_line, e_col, is_macro)) = resolve_span_coords(tcx, item_span, package_root) else {
            continue;
        };

        let mut path = rustc_middle::ty::print::with_no_visible_paths!(rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(def_id)));
        let mut name = path.rsplit("::").next().unwrap_or(&path).to_string();

        let mut signature = match kind {
            hir::def::DefKind::Fn => Some(sanitize_type_name(&tcx.fn_sig(def_id).skip_binder().to_string())),
            hir::def::DefKind::AssocFn => Some(sysroot::util::format_assoc_fn_sig(tcx, def_id.to_def_id())),
            hir::def::DefKind::Field => Some(sanitize_type_name(&tcx.type_of(def_id).skip_binder().to_string())),
            _ => None,
        };

        if kind == hir::def::DefKind::Use {
            let mut is_glob = false;
            if let hir::Node::Item(item) = tcx.hir_node_by_def_id(def_id) {
                if let hir::ItemKind::Use(use_path, use_kind) = &item.kind {
                    let target_path = use_path
                        .res
                        .iter()
                        .flatten()
                        .find_map(|r| {
                            r.opt_def_id()
                                .map(|target_did| rustc_middle::ty::print::with_no_visible_paths!(rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(target_did))))
                        })
                        .unwrap_or_else(|| use_path.segments.iter().map(|s| s.ident.as_str()).collect::<Vec<_>>().join("::"));

                    if matches!(use_kind, hir::UseKind::Glob) {
                        is_glob = true;
                        name = "*".to_string();
                        path = format!("{path}::*");
                        signature = Some(target_path);
                    } else if !target_path.is_empty() {
                        signature = Some(target_path);
                    }
                }
            }

            if !is_glob {
                if let Ok(snippet) = tcx.sess.source_map().span_to_snippet(item_span) {
                    let trimmed = snippet.trim();
                    if trimmed.ends_with("::*") || trimmed.ends_with("::*;") {
                        name = "*".to_string();
                        path = format!("{path}::*");
                    } else if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_alphanumeric() || c == '_') {
                        name = trimmed.to_string();
                        if let Some((parent, _)) = path.rsplit_once("::") {
                            path = format!("{parent}::{name}");
                        } else {
                            path = name.clone();
                        }
                    }
                }
            }
        }

        let mut owners = Vec::new();

        #[allow(clippy::collapsible_match)]
        match kind {
            hir::def::DefKind::TyParam => {
                let parent_did = tcx.parent(did);
                let param_name = tcx.item_name(did);
                let mut bounds = Vec::new();
                let mut cur_preds = Some(tcx.predicates_of(parent_did));
                while let Some(preds) = cur_preds {
                    for &(clause, _) in preds.predicates {
                        if let rustc_middle::ty::ClauseKind::Trait(trait_pred) = clause.kind().skip_binder() {
                            let self_ty = trait_pred.self_ty();
                            if let rustc_middle::ty::TyKind::Param(param_ty) = self_ty.kind() {
                                if param_ty.name == param_name {
                                    let bound_did = trait_pred.trait_ref.def_id;
                                    if tcx.lang_items().sized_trait() != Some(bound_did) && !tcx.trait_is_auto(bound_did) {
                                        let bound_path =
                                            rustc_middle::ty::print::with_no_visible_paths!(rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(bound_did)));
                                        if !bound_path.starts_with("core::marker::") && !bounds.contains(&bound_path) {
                                            bounds.push(bound_path);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    cur_preds = preds.parent.map(|p| tcx.predicates_of(p));
                }
                if !bounds.is_empty() {
                    signature = Some(bounds.join(" + "));
                }

                // Record enclosing item scope coordinates in owners
                let parent_span = if let Some(local_did) = parent_did.as_local() {
                    let hir_id = tcx.local_def_id_to_hir_id(local_did);
                    tcx.hir_span_with_body(hir_id)
                } else {
                    tcx.def_span(parent_did)
                };
                if let Some((_, p_sl, p_sc, p_el, p_ec, _)) = resolve_span_coords(tcx, parent_span, package_root) {
                    owners.push(format!("scope:{}:{}:{}:{}", p_sl, p_sc, p_el, p_ec));
                }
            }
            hir::def::DefKind::AssocFn | hir::def::DefKind::AssocConst { .. } | hir::def::DefKind::AssocTy => {
                let parent_did = tcx.parent(did);
                let parent_kind = tcx.def_kind(parent_did);
                if matches!(parent_kind, hir::def::DefKind::Impl { .. }) {
                    let self_ty = tcx.type_of(parent_did).skip_binder();
                    if let Some(owner) = resolve_owner_from_ty(tcx, self_ty, None) {
                        owners.push(owner);
                    }
                    // P3.2: Trait impl methods are indexed ONLY under the Self type.
                    // They are NOT indexed under the trait path to prevent thousands of duplicate
                    // method entries (e.g. `fmt` from every `impl Display for ...`) under `Trait::`.
                } else if parent_kind == hir::def::DefKind::Trait {
                    // Under `Trait::`, only index items declared directly in the trait definition itself.
                    let trait_path = rustc_middle::ty::print::with_no_visible_paths!(rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(parent_did)));
                    owners.push(trait_path);
                }
                if kind == hir::def::DefKind::AssocTy && parent_kind == hir::def::DefKind::Trait {
                    let mut assoc_bounds = Vec::new();
                    for (clause, _) in tcx.explicit_item_bounds(did).skip_binder() {
                        if let rustc_middle::ty::ClauseKind::Trait(trait_pred) = clause.kind().skip_binder() {
                            let bound_did = trait_pred.trait_ref.def_id;
                            if tcx.lang_items().sized_trait() != Some(bound_did) && !tcx.trait_is_auto(bound_did) {
                                let bound_path = rustc_middle::ty::print::with_no_visible_paths!(rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(bound_did)));
                                if !bound_path.starts_with("core::marker::") && !assoc_bounds.contains(&bound_path) {
                                    assoc_bounds.push(bound_path);
                                }
                            }
                        }
                    }
                    if !assoc_bounds.is_empty() {
                        signature = Some(assoc_bounds.join(" + "));
                    }
                }
            }
            hir::def::DefKind::Variant => {
                // P3.1: Enum variants are owned by the enum type path for `Enum::Variant` completion.
                let parent_did = tcx.parent(did);
                let enum_path = rustc_middle::ty::print::with_no_visible_paths!(rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(parent_did)));
                owners.push(enum_path);
            }
            hir::def::DefKind::Field => {
                let mut parent_did = tcx.parent(did);
                if tcx.def_kind(parent_did) == hir::def::DefKind::Variant {
                    parent_did = tcx.parent(parent_did);
                }
                let struct_path = rustc_middle::ty::print::with_no_visible_paths!(rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(parent_did)));
                owners.push(struct_path);
            }
            hir::def::DefKind::Fn
            | hir::def::DefKind::Struct
            | hir::def::DefKind::Enum
            | hir::def::DefKind::Mod
            | hir::def::DefKind::Const { .. }
            | hir::def::DefKind::Static { .. }
            | hir::def::DefKind::Trait
            | hir::def::DefKind::TyAlias
            | hir::def::DefKind::Union => {
                // P3.1: Items declared inside a module are owned by the module path for `module::` completion.
                if !did.is_crate_root() && tcx.def_kind(tcx.parent(did)) == hir::def::DefKind::Mod {
                    let parent_did = tcx.parent(did);
                    let mod_path = rustc_middle::ty::print::with_no_visible_paths!(rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(parent_did)));
                    owners.push(mod_path);
                }
            }
            _ => {}
        }

        // Detect Deref and trait implementations: impl Trait for T { ... }
        if matches!(kind, hir::def::DefKind::Impl { of_trait: true }) {
            let trait_ref = tcx.impl_trait_ref(did);
            let trait_did = trait_ref.skip_binder().def_id;
            let self_ty = tcx.type_of(did).skip_binder();
            let src_owner = resolve_owner_from_ty(tcx, self_ty, None);

            // 1. Deref implementation: impl Deref for T { type Target = U; }
            if tcx.lang_items().deref_trait() == Some(trait_did) {
                for &assoc_did in tcx.associated_item_def_ids(did) {
                    if tcx.def_kind(assoc_did) == hir::def::DefKind::AssocTy && tcx.item_name(assoc_did).as_str() == "Target" {
                        let target_ty = tcx.type_of(assoc_did).skip_binder();
                        let tgt_owner = resolve_owner_from_ty(tcx, target_ty, None);
                        if let (Some(src), Some(tgt)) = (src_owner.clone(), tgt_owner) {
                            visitor.writer.add_deref(src, tgt);
                        }
                    }
                }
            }

            // 2. Trait implementation link: connect type to implemented trait
            // (e.g. MyIter -> core::iter::traits::iterator::Iterator, MyType -> core::fmt::Display)
            if tcx.lang_items().sized_trait() != Some(trait_did) && !tcx.trait_is_auto(trait_did) && !tcx.associated_item_def_ids(trait_did).is_empty() {
                let trait_path = rustc_middle::ty::print::with_no_visible_paths!(rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(trait_did)));
                if !trait_path.starts_with("core::marker::") {
                    if let Some(src) = src_owner {
                        if src != trait_path {
                            visitor.writer.add_deref(src, trait_path);
                        }
                    }
                }
            }
        }

        // Index supertrait relationships in the autoderef chain: trait SubTrait: SuperTrait
        if kind == hir::def::DefKind::Trait {
            let trait_path = rustc_middle::ty::print::with_no_visible_paths!(rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(did)));
            for (clause, _) in tcx.explicit_super_predicates_of(did).skip_binder() {
                if let rustc_middle::ty::ClauseKind::Trait(trait_pred) = clause.kind().skip_binder() {
                    let super_did = trait_pred.trait_ref.def_id;
                    if super_did != did && tcx.lang_items().sized_trait() != Some(super_did) && !tcx.trait_is_auto(super_did) {
                        let super_path = rustc_middle::ty::print::with_no_visible_paths!(rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(super_did)));
                        if !super_path.starts_with("core::marker::") {
                            visitor.writer.add_deref(trait_path.clone(), super_path);
                        }
                    }
                }
            }
        }

        let def_idx = visitor.writer.add_definition_with_owners(
            RawDefinition {
                def_id: def_id.local_def_index.as_u32(),
                kind: map_def_kind(kind),
                visibility: 0,
                is_macro_expansion: is_macro,
                name,
                path,
                signature,
                file,
                start_line: s_line,
                start_col: s_col,
                end_line: e_line,
                end_col: e_col,
            },
            owners,
        );
        local_def_indices.insert(did, def_idx);
        match kind {
            hir::def::DefKind::Fn | hir::def::DefKind::AssocFn => callables.push((did, def_idx)),
            hir::def::DefKind::Struct | hir::def::DefKind::Union | hir::def::DefKind::Enum => adts.push((did, def_idx)),
            hir::def::DefKind::TyAlias | hir::def::DefKind::Const { .. } | hir::def::DefKind::Static { .. } | hir::def::DefKind::AssocConst { .. } | hir::def::DefKind::Variant => {
                def_tys.push((did, def_idx))
            }
            _ => {}
        }
    }

    if !callables.is_empty() || !impl_dids.is_empty() || !adts.is_empty() || !def_tys.is_empty() {
        let mut lowerer = crate::ty_lower::TcxTypeLowerer::new(tcx, rustc_span::def_id::LOCAL_CRATE, &visitor.krate_to_unit, &local_def_indices);
        lowerer.record_fn_sigs(&callables, &mut visitor.writer);
        lowerer.record_impls(&impl_dids, &mut visitor.writer);
        lowerer.record_adts(&adts, &mut visitor.writer);
        lowerer.record_def_tys(&def_tys, &mut visitor.writer);
    }

    // Traverse all item bodies for references and types
    tcx.hir_visit_all_item_likes_in_crate(visitor);
}
