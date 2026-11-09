/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Materializes sysroot `DefEntry` rows for `#[lang]` items and associated items of lang traits
//! that were skipped by the stable public module walk (e.g. `Try`, `FromResidual`, `Residual`,
//! `Fn::call`, `FnMut::call_mut`, `FnOnce::call_once`, `AsyncFn::async_call`, `Receiver`).

use std::collections::HashSet;
use std::path::Path;

use ridx_format::{IndexWriter, LangItemKind, RawDefinition};
use rustc_hir::def::DefKind;
use rustc_middle::ty::TyCtxt;
use rustc_span::def_id::{CrateNum, DefId};

use super::{items, SysrootCollector};
use crate::map_def_kind;
use crate::sanitize::sanitize_type_name;

fn emit_missing_sysroot_def<'tcx>(
    tcx: TyCtxt<'tcx>,
    did: DefId,
    sysroot_src: Option<&Path>,
    writer: &mut IndexWriter,
    visited: &mut HashSet<DefId>,
    collector: &mut SysrootCollector,
) -> bool {
    if collector.local_def_indices.contains_key(&did) {
        return false;
    }
    visited.insert(did);
    let kind = tcx.def_kind(did);
    let path = rustc_middle::ty::print::with_no_visible_paths!(rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(did)));
    let name = tcx.opt_item_name(did).map(|s| s.to_string()).unwrap_or_else(|| path.rsplit("::").next().unwrap_or(&path).to_string());
    let signature = match kind {
        DefKind::Fn | DefKind::AssocFn => Some(sanitize_type_name(&tcx.fn_sig(did).skip_binder().to_string())),
        DefKind::Field => Some(sanitize_type_name(&tcx.type_of(did).skip_binder().to_string())),
        _ => None,
    };
    let (file, start_line, start_col, end_line, end_col) = items::extract_def_span(tcx, did, sysroot_src);

    // Do NOT register unstable/private lang items into legacy v3 `defs_by_owner` or `derefs`:
    // Phase 2/3 live type inference accesses them strictly via v5 `LangItemEntry` / `def_index`,
    // whereas `defs_by_owner` / `derefs` feed heuristic completion on stable Rust code.
    let def_idx = writer.add_definition_unowned(RawDefinition {
        def_id: collector.total_definitions as u32,
        kind: map_def_kind(kind),
        visibility: crate::ty_lower::fields::def_visibility(tcx, did, kind) as u8,
        is_macro_expansion: false,
        name,
        path,
        signature,
        file,
        start_line,
        start_col,
        end_line,
        end_col,
    });
    collector.register_primary(did, kind, def_idx);
    true
}

/// Ensures every lang item in `lang_items` — and every associated item of newly materialized
/// unstable lang-item `Trait`s — has a local `DefEntry` and is registered in `collector` for
/// `FnSig`, `Field`, `DefTy`, `GenericParam`, and `WherePred` lowering regardless of
/// `is_stable_api` / `is_publicly_visible`.
pub fn materialize_lang_items<'tcx>(
    tcx: TyCtxt<'tcx>,
    lang_items: &[(LangItemKind, DefId)],
    sysroot_src: Option<&Path>,
    writer: &mut IndexWriter,
    visited: &mut HashSet<DefId>,
    collector: &mut SysrootCollector,
) {
    let mut newly_added_traits = HashSet::new();
    for &(_kind, did) in lang_items {
        let added = emit_missing_sysroot_def(tcx, did, sysroot_src, writer, visited, collector);
        if added && tcx.def_kind(did) == DefKind::Trait {
            newly_added_traits.insert(did);
        }
    }

    let mut seen_traits = HashSet::new();
    for &(_kind, did) in lang_items {
        if !newly_added_traits.contains(&did) || !seen_traits.insert(did) {
            continue;
        }
        for &assoc_did in tcx.associated_item_def_ids(did) {
            let Some(sym) = tcx.opt_item_name(assoc_did) else {
                continue;
            };
            let s = sym.as_str();
            if s.starts_with("__rust_") || s.starts_with("__") {
                continue;
            }
            emit_missing_sysroot_def(tcx, assoc_did, sysroot_src, writer, visited, collector);
        }
    }
}

/// Returns `true` when `did` is an unstable sysroot type or trait that appears in stable generic
/// parameter defaults (`alloc::alloc::Global` and `core::alloc::Allocator` on `Vec<T, A = Global>`,
/// `Box<T, A = Global>`, etc.) and therefore must be materialized in `core.ridx` / `alloc.ridx`.
#[inline]
pub fn is_materialized_unstable_type(tcx: TyCtxt<'_>, did: DefId) -> bool {
    if !matches!(tcx.crate_name(did.krate).as_str(), "core" | "alloc") {
        return false;
    }
    let Some(sym) = tcx.opt_item_name(did) else {
        return false;
    };
    if !matches!(sym.as_str(), "Global" | "Allocator") {
        return false;
    }
    let path = rustc_middle::ty::print::with_no_visible_paths!(rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(did)));
    matches!(path.as_str(), "alloc::alloc::Global" | "core::alloc::Allocator" | "alloc::alloc::Allocator")
}

/// Materializes `core::alloc::Allocator` (`core.ridx`) and `alloc::alloc::Global` (`alloc.ridx`),
/// so stable container types (`Vec<T, Global>`, `Box<T, Global>`, etc.) lower their default
/// allocator parameter and `A: Allocator` bounds cleanly rather than to `RawTy::Unknown`.
pub fn materialize_unstable_default_types<'tcx>(
    tcx: TyCtxt<'tcx>,
    target_crate: &str,
    cnum: CrateNum,
    sysroot_src: Option<&Path>,
    writer: &mut IndexWriter,
    visited: &mut HashSet<DefId>,
    collector: &mut SysrootCollector,
) {
    if !matches!(target_crate, "core" | "alloc") {
        return;
    }
    let Some(alloc_mod) = tcx.module_children(cnum.as_def_id()).iter().find_map(|c| (c.ident.as_str() == "alloc").then(|| c.res.opt_def_id()).flatten()) else {
        return;
    };
    for child in tcx.module_children(alloc_mod) {
        if !matches!(child.ident.as_str(), "Global" | "Allocator") {
            continue;
        }
        let Some(did) = child.res.opt_def_id().filter(|d| d.krate == cnum) else {
            continue;
        };
        emit_missing_sysroot_def(tcx, did, sysroot_src, writer, visited, collector);
    }
}
