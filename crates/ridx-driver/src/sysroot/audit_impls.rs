/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Path grouping, primitive inherent-impl, and trait-impl helpers for sysroot completeness audit.

use std::collections::{BTreeMap, HashSet, VecDeque};

use rustc_hir::def::DefKind;
use rustc_middle::ty::fast_reject::SimplifiedType;
use rustc_middle::ty::{FloatTy, IntTy, Mutability, TyCtxt, TyKind, UintTy};
use rustc_span::def_id::{CrateNum, DefId};

use super::audit_types::{CrateCompleteness, FamilyCompleteness};
use super::util::{is_publicly_visible, is_stable_api};

pub(super) fn canon_path(tcx: TyCtxt<'_>, did: DefId) -> String {
    rustc_middle::ty::print::with_no_visible_paths!(rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(did)))
}

pub(super) fn family_key(cname: &str, path: &str) -> String {
    let rest = path.strip_prefix(cname).and_then(|s| s.strip_prefix("::")).unwrap_or("");
    let top = rest.split("::").next().unwrap_or("");
    if top.is_empty() {
        format!("{cname}::<root>")
    } else {
        format!("{cname}::{top}")
    }
}

pub(super) fn subfamily_key(cname: &str, cpath: &str, mod_path: &str) -> Option<String> {
    let p = if cpath.starts_with("core::ops::")
        || cpath.starts_with("core::fmt")
        || cpath.starts_with("core::iter::")
        || cpath.starts_with("alloc::collections::")
        || cpath.starts_with("std::io")
        || cpath.starts_with("std::collections::")
    {
        cpath
    } else if mod_path.starts_with("std::collections") || mod_path.starts_with("alloc::collections") || mod_path.starts_with("std::io") {
        mod_path
    } else {
        return None;
    };
    let rest = p.strip_prefix(cname).and_then(|s| s.strip_prefix("::")).unwrap_or("");
    let mut segs = rest.split("::");
    let s1 = segs.next().unwrap_or("");
    let s2 = segs.next().unwrap_or("");
    if s1.is_empty() {
        None
    } else if matches!(s1, "ops" | "iter" | "collections" | "io") && !s2.is_empty() {
        Some(format!("{cname}::{s1}::{s2}"))
    } else {
        Some(format!("{cname}::{s1}"))
    }
}

pub(super) fn count_subtree_defs(tcx: TyCtxt<'_>, mod_did: DefId) -> usize {
    let mut q = VecDeque::from([mod_did]);
    let mut seen_m = HashSet::from([mod_did]);
    let mut defs = HashSet::new();
    while let Some(m) = q.pop_front() {
        for ch in tcx.module_children(m) {
            if !ch.vis.is_public() {
                continue;
            }
            let Some(did) = ch.res.opt_def_id() else { continue };
            if did.krate == mod_did.krate {
                defs.insert(did);
                if tcx.def_kind(did) == DefKind::Mod && seen_m.insert(did) {
                    q.push_back(did);
                }
            }
        }
    }
    defs.len()
}

pub(super) fn audit_primitive_impls(
    tcx: TyCtxt<'_>,
    cname: &str,
    cnum: CrateNum,
    ridx_canon: &HashSet<String>,
    seen_did: &mut HashSet<DefId>,
    cc: &mut CrateCompleteness,
    families: &mut BTreeMap<String, FamilyCompleteness>,
) {
    let prims = [
        SimplifiedType::Str,
        SimplifiedType::Slice,
        SimplifiedType::Bool,
        SimplifiedType::Char,
        SimplifiedType::Uint(UintTy::U8),
        SimplifiedType::Uint(UintTy::U16),
        SimplifiedType::Uint(UintTy::U32),
        SimplifiedType::Uint(UintTy::U64),
        SimplifiedType::Uint(UintTy::U128),
        SimplifiedType::Uint(UintTy::Usize),
        SimplifiedType::Int(IntTy::I8),
        SimplifiedType::Int(IntTy::I16),
        SimplifiedType::Int(IntTy::I32),
        SimplifiedType::Int(IntTy::I64),
        SimplifiedType::Int(IntTy::I128),
        SimplifiedType::Int(IntTy::Isize),
        SimplifiedType::Float(FloatTy::F32),
        SimplifiedType::Float(FloatTy::F64),
        SimplifiedType::Array,
        SimplifiedType::Ptr(Mutability::Not),
        SimplifiedType::Ptr(Mutability::Mut),
    ];
    for simp in prims {
        for &imp in tcx.incoherent_impls(simp) {
            if imp.krate != cnum {
                continue;
            }
            for &a_did in tcx.associated_item_def_ids(imp) {
                if !is_publicly_visible(tcx, a_did) || !is_stable_api(tcx, a_did) || !seen_did.insert(a_did) {
                    continue;
                }
                if tcx.opt_item_name(a_did).is_none_or(|s| s.as_str().starts_with("__")) {
                    continue;
                }
                let ap = canon_path(tcx, a_did);
                let fk = family_key(cname, &ap);
                let fam = families.entry(fk.clone()).or_insert_with(|| FamilyCompleteness { family: fk, ..Default::default() });
                cc.tcx_pub_stable_defs += 1;
                fam.pub_stable_total += 1;
                if ridx_canon.contains(&ap) {
                    cc.tcx_pub_stable_in_ridx += 1;
                    fam.pub_stable_in_ridx += 1;
                }
            }
        }
    }
}

pub(super) fn audit_trait_impls(
    tcx: TyCtxt<'_>,
    cnum: CrateNum,
    all_pub_traits: &[(DefId, String, bool)],
    cc: &mut CrateCompleteness,
    families: &mut BTreeMap<String, FamilyCompleteness>,
    subfamilies: &mut BTreeMap<String, FamilyCompleteness>,
) {
    let mut seen_impls = HashSet::new();
    for (t_did, t_path, trait_dropped) in all_pub_traits {
        let t_crate = t_path.split("::").next().unwrap_or("core");
        let tfk = family_key(t_crate, t_path);
        let tsfk = subfamily_key(t_crate, t_path, t_path);
        for impl_did in tcx.all_impls(*t_did) {
            if impl_did.krate != cnum || !seen_impls.insert(impl_did) {
                continue;
            }
            cc.trait_impl_blocks_missing += 1;
            let self_ty = tcx.type_of(impl_did).skip_binder();
            if *trait_dropped {
                cc.impl_marker_or_unstable_trait += 1;
            } else {
                match self_ty.kind() {
                    TyKind::Param(_) => cc.impl_blanket_param += 1,
                    TyKind::Ref(..) => cc.impl_ref_collapsed += 1,
                    TyKind::Tuple(_) | TyKind::FnPtr(..) | TyKind::Never => cc.impl_tuple_fn_never += 1,
                    _ => cc.impl_adt_or_prim += 1,
                }
            }
            let mut atys = 0usize;
            for &assoc_did in tcx.associated_item_def_ids(impl_did) {
                match tcx.def_kind(assoc_did) {
                    DefKind::AssocTy => {
                        cc.trait_impl_assoc_tys_missing += 1;
                        atys += 1;
                    }
                    DefKind::AssocFn => cc.trait_impl_assoc_fns_missing += 1,
                    _ => {}
                }
            }
            let tfam = families.entry(tfk.clone()).or_insert_with(|| FamilyCompleteness { family: tfk.clone(), ..Default::default() });
            tfam.trait_impls_of_family_traits += 1;
            tfam.trait_impl_assoc_tys_of_family_traits += atys;
            if let Some(ref sk) = tsfk {
                let sfam = subfamilies.entry(sk.clone()).or_insert_with(|| FamilyCompleteness { family: sk.clone(), ..Default::default() });
                sfam.trait_impls_of_family_traits += 1;
                sfam.trait_impl_assoc_tys_of_family_traits += atys;
            }
        }
    }
}
