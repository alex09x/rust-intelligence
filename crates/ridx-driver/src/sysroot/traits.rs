/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use ridx_format::IndexWriter;
use rustc_hir::def::DefKind;
use rustc_middle::ty::TyCtxt;
use rustc_span::def_id::{CrateNum, DefId};

use super::util::{is_publicly_visible, is_stable_api};
use crate::owner::resolve_owner_from_ty;

/// Collects all public stable traits declared in sysroot crates (`core`, `alloc`, `std`)
/// that declare associated items/methods and are not marker/auto traits.
pub fn collect_sysroot_traits<'tcx>(tcx: TyCtxt<'tcx>) -> Vec<(DefId, String)> {
    let mut traits = Vec::new();
    let mut visited = std::collections::HashSet::new();

    for &cnum in tcx.crates(()) {
        let cname_sym = tcx.crate_name(cnum);
        let cname = cname_sym.as_str();
        if cname != "core" && cname != "alloc" && cname != "std" {
            continue;
        }

        let mut queue = std::collections::VecDeque::new();
        let root_did = cnum.as_def_id();
        queue.push_back(root_did);
        visited.insert(root_did);

        while let Some(m_did) = queue.pop_front() {
            for child in tcx.module_children(m_did) {
                let Some(did) = child.res.opt_def_id() else {
                    continue;
                };
                if did.krate != cnum || !visited.insert(did) {
                    continue;
                }
                let kind = tcx.def_kind(did);
                if kind == DefKind::Mod && is_publicly_visible(tcx, did) {
                    queue.push_back(did);
                } else if kind == DefKind::Trait {
                    if !is_publicly_visible(tcx, did) || !is_stable_api(tcx, did) {
                        continue;
                    }
                    if tcx.lang_items().sized_trait() == Some(did) || tcx.trait_is_auto(did) {
                        continue;
                    }
                    if tcx.associated_item_def_ids(did).is_empty() {
                        continue;
                    }
                    let path = rustc_middle::ty::print::with_no_visible_paths!(
                        rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(did))
                    );
                    if path.starts_with("core::marker::") {
                        continue;
                    }
                    traits.push((did, path));
                }
            }
        }
    }
    traits
}

/// Extracts Deref edges (e.g. `String -> str`, `Vec<T> -> [T]`) and trait implementation links
/// (`Option -> Clone`, `Option -> IntoIterator`, `Vec -> Extend`) for types defined in `cnum`.
pub fn extract_trait_and_deref_links<'tcx>(
    tcx: TyCtxt<'tcx>,
    cnum: CrateNum,
    traits: &[(DefId, String)],
    writer: &mut IndexWriter,
    total_derefs: &mut usize,
) {
    let deref_trait_did = tcx.lang_items().deref_trait();

    let mut seen_derefs = std::collections::HashSet::new();

    // 1. Deref impls: map source type to Deref::Target
    if let Some(deref_did) = deref_trait_did {
        let trait_impls = tcx.trait_impls_of(deref_did);
        for &impl_did in trait_impls.blanket_impls() {
            process_deref_impl(tcx, cnum, impl_did, writer, &mut seen_derefs, total_derefs);
        }
        for (_, impl_dids) in trait_impls.non_blanket_impls() {
            for &impl_did in impl_dids {
                process_deref_impl(tcx, cnum, impl_did, writer, &mut seen_derefs, total_derefs);
            }
        }
    }

    // 2. Trait-impl links: connect type to implemented traits
    for &(trait_did, ref trait_path) in traits {
        if Some(trait_did) == deref_trait_did {
            continue;
        }

        let trait_impls = tcx.trait_impls_of(trait_did);
        for &impl_did in trait_impls.blanket_impls() {
            process_trait_type_link(
                tcx,
                cnum,
                impl_did,
                trait_path,
                writer,
                &mut seen_derefs,
                total_derefs,
            );
        }
        for (_, impl_dids) in trait_impls.non_blanket_impls() {
            for &impl_did in impl_dids {
                process_trait_type_link(
                    tcx,
                    cnum,
                    impl_did,
                    trait_path,
                    writer,
                    &mut seen_derefs,
                    total_derefs,
                );
            }
        }
    }
}

fn process_deref_impl<'tcx>(
    tcx: TyCtxt<'tcx>,
    cnum: CrateNum,
    impl_did: DefId,
    writer: &mut IndexWriter,
    seen_derefs: &mut std::collections::HashSet<(String, String)>,
    total_derefs: &mut usize,
) {
    if impl_did.krate != cnum {
        return;
    }
    let self_ty = tcx.type_of(impl_did).skip_binder();
    let Some(src_owner) = resolve_owner_from_ty(tcx, self_ty, None) else {
        return;
    };
    for &assoc_did in tcx.associated_item_def_ids(impl_did) {
        if tcx.def_kind(assoc_did) == DefKind::AssocTy
            && tcx
                .opt_item_name(assoc_did)
                .is_some_and(|s| s.as_str() == "Target")
        {
            let target_ty = tcx.type_of(assoc_did).skip_binder();
            if let Some(tgt_owner) = resolve_owner_from_ty(tcx, target_ty, None) {
                if src_owner != tgt_owner
                    && seen_derefs.insert((src_owner.clone(), tgt_owner.clone()))
                {
                    writer.add_deref(src_owner.clone(), tgt_owner);
                    *total_derefs += 1;
                }
            }
        }
    }
}

fn process_trait_type_link<'tcx>(
    tcx: TyCtxt<'tcx>,
    cnum: CrateNum,
    impl_did: DefId,
    trait_path: &str,
    writer: &mut IndexWriter,
    seen_derefs: &mut std::collections::HashSet<(String, String)>,
    total_derefs: &mut usize,
) {
    if impl_did.krate != cnum {
        return;
    }
    let self_ty = tcx.type_of(impl_did).skip_binder();
    let Some(src_owner) = resolve_owner_from_ty(tcx, self_ty, None) else {
        return;
    };
    if src_owner != trait_path && seen_derefs.insert((src_owner.clone(), trait_path.to_string())) {
        writer.add_deref(src_owner, trait_path);
        *total_derefs += 1;
    }
}
