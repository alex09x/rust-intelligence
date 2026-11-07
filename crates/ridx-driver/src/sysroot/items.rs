/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use std::collections::{HashMap, HashSet};
use std::path::Path;

use ridx_format::{IndexWriter, RawDefinition};
use rustc_hir::def::DefKind;
use rustc_middle::ty::TyCtxt;
use rustc_span::def_id::DefId;

use super::util::{self, is_publicly_visible, is_stable_api, remap_sysroot_path};
use super::SysrootCollector;
use crate::map_def_kind;
use crate::sanitize::sanitize_type_name;

/// Extracts the diagnostic-remapped source span for a compiler definition.
pub fn extract_def_span(tcx: TyCtxt<'_>, did: DefId, sysroot_src: Option<&Path>) -> (String, u32, u32, u32, u32) {
    let item_span = tcx.def_ident_span(did).unwrap_or_else(|| tcx.def_span(did));
    if !item_span.is_dummy() {
        let sm = tcx.sess.source_map();
        let lo = sm.lookup_char_pos(item_span.lo());
        let hi = sm.lookup_char_pos(item_span.hi());
        let raw_file = sm.filename_for_diagnostics(&lo.file.name).to_string();
        let remapped = remap_sysroot_path(&raw_file, sysroot_src);
        (remapped, lo.line as u32, (lo.col.0 + 1) as u32, hi.line as u32, (hi.col.0 + 1) as u32)
    } else {
        (String::new(), 0, 0, 0, 0)
    }
}

/// Records a lightweight re-export alias in the target crate index without duplicating item bodies.
pub fn record_reexport<'tcx>(
    tcx: TyCtxt<'tcx>,
    did: DefId,
    name_str: &str,
    mod_path: &str,
    sysroot_src: Option<&Path>,
    writer: &mut IndexWriter,
    collector: &mut SysrootCollector,
) {
    let canonical_path = rustc_middle::ty::print::with_no_visible_paths!(rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(did)));
    let alias_path = format!("{mod_path}::{name_str}");
    let (file, s_line, s_col, e_line, e_col) = extract_def_span(tcx, did, sysroot_src);
    let kind = tcx.def_kind(did);

    let raw_def = RawDefinition {
        def_id: collector.total_definitions as u32,
        kind: map_def_kind(kind),
        visibility: 0,
        is_macro_expansion: false,
        name: name_str.to_string(),
        path: alias_path,
        signature: Some(canonical_path),
        file,
        start_line: s_line,
        start_col: s_col,
        end_line: e_line,
        end_col: e_col,
    };

    let def_idx = writer.add_reexport_with_owners(raw_def, vec![mod_path.to_string()]);
    collector.register_reexport(did, kind, def_idx);
}

/// Extracts supertraits and associated items for a trait.
#[allow(clippy::too_many_arguments)]
pub fn extract_trait_items<'tcx>(
    tcx: TyCtxt<'tcx>,
    did: DefId,
    path: &str,
    sysroot_src: Option<&Path>,
    writer: &mut IndexWriter,
    visited: &mut HashSet<DefId>,
    collector: &mut SysrootCollector,
    total_derefs: &mut usize,
) {
    for (clause, _) in tcx.explicit_super_predicates_of(did).skip_binder() {
        if let rustc_middle::ty::ClauseKind::Trait(trait_pred) = clause.kind().skip_binder() {
            let super_did = trait_pred.trait_ref.def_id;
            if super_did != did && tcx.lang_items().sized_trait() != Some(super_did) && !tcx.trait_is_auto(super_did) {
                let super_path = rustc_middle::ty::print::with_no_visible_paths!(rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(super_did)));
                if !super_path.starts_with("core::marker::") {
                    writer.add_deref(path.to_string(), super_path);
                    *total_derefs += 1;
                }
            }
        }
    }

    for assoc_did in tcx.associated_item_def_ids(did) {
        let assoc_did = *assoc_did;
        if !visited.insert(assoc_did) {
            continue;
        }
        if !is_publicly_visible(tcx, assoc_did) || !is_stable_api(tcx, assoc_did) {
            continue;
        }
        let Some(name_sym) = tcx.opt_item_name(assoc_did) else {
            continue;
        };
        let a_name = name_sym.to_string();
        if a_name.starts_with("__rust_") || a_name.starts_with("__") {
            continue;
        }
        let a_kind = tcx.def_kind(assoc_did);
        let a_path = rustc_middle::ty::print::with_no_visible_paths!(rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(assoc_did)));
        let mut a_sig = match a_kind {
            DefKind::Fn | DefKind::AssocFn => Some(sanitize_type_name(&tcx.fn_sig(assoc_did).skip_binder().to_string())),
            _ => None,
        };
        let mut a_owners = vec![path.to_string()];
        if a_kind == DefKind::AssocTy {
            let mut assoc_bounds = Vec::new();
            for (clause, _) in tcx.explicit_item_bounds(assoc_did).skip_binder() {
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
                a_sig = Some(assoc_bounds.join(" + "));
                for b in assoc_bounds {
                    if !a_owners.contains(&b) {
                        a_owners.push(b);
                    }
                }
            }
        }
        let (a_file, a_sl, a_sc, a_el, a_ec) = extract_def_span(tcx, assoc_did, sysroot_src);

        let def_idx = writer.add_definition_with_owners(
            RawDefinition {
                def_id: collector.total_definitions as u32,
                kind: map_def_kind(a_kind),
                visibility: 0,
                is_macro_expansion: false,
                name: a_name,
                path: a_path,
                signature: a_sig,
                file: a_file,
                start_line: a_sl,
                start_col: a_sc,
                end_line: a_el,
                end_col: a_ec,
            },
            a_owners,
        );
        collector.register_primary(assoc_did, a_kind, def_idx);
    }
}

fn is_applicable_inherent_impl<'tcx>(impl_ty: rustc_middle::ty::Ty<'tcx>, ty: rustc_middle::ty::Ty<'tcx>, adt_did: DefId) -> bool {
    if impl_ty == ty {
        return true;
    }
    match (impl_ty.kind(), ty.kind()) {
        (rustc_middle::ty::TyKind::Adt(i_adt, i_args), rustc_middle::ty::TyKind::Adt(t_adt, t_args)) => {
            if i_adt.did() != adt_did || t_adt.did() != adt_did || i_args.len() != t_args.len() {
                return false;
            }
            i_args.iter().zip(t_args.iter()).all(|(ia, ta)| {
                if ia == ta {
                    return true;
                }
                if let Some(it) = ia.as_type() {
                    matches!(it.kind(), rustc_middle::ty::TyKind::Param(_))
                } else {
                    false
                }
            })
        }
        _ => false,
    }
}

/// Extracts inherent methods and constants for struct, enum, union, and type aliases.
pub fn extract_inherent_impl_items<'tcx>(
    tcx: TyCtxt<'tcx>,
    did: DefId,
    path: &str,
    sysroot_src: Option<&Path>,
    writer: &mut IndexWriter,
    assoc_visited: &mut HashMap<DefId, u32>,
    collector: &mut SysrootCollector,
) {
    let (impl_dids, adt_path): (Vec<DefId>, Option<String>) = match tcx.def_kind(did) {
        DefKind::TyAlias => {
            let ty = tcx.type_of(did).skip_binder();
            if let Some(adt_def) = ty.ty_adt_def() {
                let a_path = rustc_middle::ty::print::with_no_visible_paths!(rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(adt_def.did())));
                let impls: Vec<DefId> = tcx
                    .inherent_impls(adt_def.did())
                    .iter()
                    .copied()
                    .filter(|&impl_did| {
                        let impl_ty = tcx.type_of(impl_did).skip_binder();
                        is_applicable_inherent_impl(impl_ty, ty, adt_def.did())
                    })
                    .collect();
                (impls, Some(a_path))
            } else {
                (Vec::new(), None)
            }
        }
        _ => (tcx.inherent_impls(did).to_vec(), None),
    };

    for impl_did in impl_dids {
        if impl_did.krate == did.krate {
            collector.register_impl(impl_did);
        }
        for assoc_did in tcx.associated_item_def_ids(impl_did) {
            let assoc_did = *assoc_did;
            if let Some(&existing_idx) = assoc_visited.get(&assoc_did) {
                writer.add_def_owner_by_index(existing_idx, path.to_string());
                continue;
            }
            if !is_publicly_visible(tcx, assoc_did) || !is_stable_api(tcx, assoc_did) {
                continue;
            }
            let Some(name_sym) = tcx.opt_item_name(assoc_did) else {
                continue;
            };
            let a_name = name_sym.to_string();
            if a_name.starts_with("__rust_") || a_name.starts_with("__") {
                continue;
            }
            let a_kind = tcx.def_kind(assoc_did);
            let a_path = rustc_middle::ty::print::with_no_visible_paths!(rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(assoc_did)));
            let a_sig = match a_kind {
                DefKind::Fn => Some(sanitize_type_name(&tcx.fn_sig(assoc_did).skip_binder().to_string())),
                DefKind::AssocFn => Some(util::format_assoc_fn_sig(tcx, assoc_did)),
                _ => None,
            };
            let (a_file, a_sl, a_sc, a_el, a_ec) = extract_def_span(tcx, assoc_did, sysroot_src);

            let mut a_owners = vec![path.to_string()];
            if let Some(ref ap) = adt_path {
                if ap != path && !a_owners.contains(ap) {
                    a_owners.push(ap.clone());
                }
            }

            let def_id = collector.total_definitions as u32;
            let def_idx = writer.add_definition_with_owners(
                RawDefinition {
                    def_id,
                    kind: map_def_kind(a_kind),
                    visibility: 0,
                    is_macro_expansion: false,
                    name: a_name,
                    path: a_path,
                    signature: a_sig,
                    file: a_file,
                    start_line: a_sl,
                    start_col: a_sc,
                    end_line: a_el,
                    end_col: a_ec,
                },
                a_owners,
            );
            assoc_visited.insert(assoc_did, def_idx);
            collector.register_primary(assoc_did, a_kind, def_idx);
        }
    }
}

/// Extracts variants for an enum definition, registering each variant with the enum path as owner.
pub fn extract_enum_variants<'tcx>(
    tcx: TyCtxt<'tcx>,
    did: DefId,
    path: &str,
    sysroot_src: Option<&Path>,
    writer: &mut IndexWriter,
    visited: &mut HashSet<DefId>,
    collector: &mut SysrootCollector,
) {
    let adt_def = tcx.adt_def(did);
    for variant in adt_def.variants() {
        let var_did = variant.def_id;
        if !visited.insert(var_did) {
            continue;
        }
        let v_name = variant.name.to_string();
        let v_path = rustc_middle::ty::print::with_no_visible_paths!(rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(var_did)));
        let (v_file, v_sl, v_sc, v_el, v_ec) = extract_def_span(tcx, var_did, sysroot_src);
        let def_idx = writer.add_definition_with_owners(
            RawDefinition {
                def_id: collector.total_definitions as u32,
                kind: map_def_kind(DefKind::Variant),
                visibility: 0,
                is_macro_expansion: false,
                name: v_name,
                path: v_path,
                signature: None,
                file: v_file,
                start_line: v_sl,
                start_col: v_sc,
                end_line: v_el,
                end_col: v_ec,
            },
            vec![path.to_string()],
        );
        collector.register_primary(var_did, DefKind::Variant, def_idx);
    }
}
