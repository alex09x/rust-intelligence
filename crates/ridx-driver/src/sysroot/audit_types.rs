/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Report structures and text formatter for `ridx-driver --audit-sysroot`.

/// Per-crate completeness comparison between `rustc` (`TyCtxt`) and `extract_sysroot_crate`.
#[derive(Debug, Clone, Default)]
pub struct CrateCompleteness {
    pub crate_name: String,
    pub ridx_total_defs: usize,
    pub ridx_canonical_defs: usize,
    pub ridx_reexport_defs: usize,
    pub ridx_deref_edges: usize,
    pub tcx_pub_stable_defs: usize,
    pub tcx_pub_stable_in_ridx: usize,
    pub tcx_pub_stable_missing: Vec<String>,
    pub skipped_arch_intrinsics_defs: usize,
    pub skipped_unstable_pub_defs: usize,
    pub unstable_traits_sample: Vec<String>,
    pub unstable_ops_assoc_sample: Vec<String>,
    pub skipped_doc_hidden_or_priv_defs: usize,
    pub skipped_foreign_krate_reexports: usize,
    pub foreign_krates: Vec<String>,
    pub foreign_reexport_samples: Vec<String>,
    pub pub_named_fields_missing: usize,
    pub pub_tuple_fields_missing: usize,
    pub pub_generic_params_missing: usize,
    pub marker_or_empty_traits: Vec<String>,
    pub trait_impl_blocks_missing: usize,
    pub impl_adt_or_prim: usize,
    pub impl_blanket_param: usize,
    pub impl_ref_collapsed: usize,
    pub impl_tuple_fn_never: usize,
    pub impl_marker_or_unstable_trait: usize,
    pub trait_impl_assoc_tys_missing: usize,
    pub trait_impl_assoc_fns_missing: usize,
}

/// Per-module-family (`<crate>::<top_mod>` or focus subfamily) completeness breakdown.
#[derive(Debug, Clone, Default)]
pub struct FamilyCompleteness {
    pub family: String,
    pub pub_stable_total: usize,
    pub pub_stable_in_ridx: usize,
    pub pub_unstable: usize,
    pub doc_hidden: usize,
    pub foreign_reexp: usize,
    pub pub_fields_missing: usize,
    pub marker_or_empty_traits: usize,
    pub trait_impls_of_family_traits: usize,
    pub trait_impl_assoc_tys_of_family_traits: usize,
}

/// Full completeness report across `core`, `alloc`, `std`.
#[derive(Debug, Clone, Default)]
pub struct SysrootCompletenessReport {
    pub crates: Vec<CrateCompleteness>,
    pub families: Vec<FamilyCompleteness>,
    pub subfamilies: Vec<FamilyCompleteness>,
}

/// Prints a structured summary of `SysrootCompletenessReport` for phase 0.3 reporting.
pub fn print_sysroot_completeness(rep: &SysrootCompletenessReport) {
    println!("=== SYSROOT COMPLETENESS (rustc TyCtxt vs .ridx) ===");
    for c in &rep.crates {
        println!(
            "[{}] ridx_defs={} (canon={}, reexp={}, derefs={}) | tcx_pub_stable={}/{} (missing={:?})",
            c.crate_name,
            c.ridx_total_defs,
            c.ridx_canonical_defs,
            c.ridx_reexport_defs,
            c.ridx_deref_edges,
            c.tcx_pub_stable_in_ridx,
            c.tcx_pub_stable_defs,
            c.tcx_pub_stable_missing,
        );
        println!(
            "  skipped_in_walk: arch_intrinsics={} unstable_pub={} doc_hidden={} foreign_reexp={} ({:?})",
            c.skipped_arch_intrinsics_defs, c.skipped_unstable_pub_defs, c.skipped_doc_hidden_or_priv_defs, c.skipped_foreign_krate_reexports, c.foreign_krates,
        );
        if !c.foreign_reexport_samples.is_empty() {
            println!("  foreign_reexport_samples: {}", c.foreign_reexport_samples.join(", "));
        }
        println!("  missing_subitems: named_fields={} tuple_fields={} generic_params={}", c.pub_named_fields_missing, c.pub_tuple_fields_missing, c.pub_generic_params_missing,);
        println!(
            "  missing_trait_impls: total_blocks={} (adt_or_prim={}, blanket_param_dropped={}, ref_collapsed={}, tuple_fn_never_dropped={}, marker_or_unstable_dropped={}) | assoc_tys={} assoc_fns={}",
            c.trait_impl_blocks_missing,
            c.impl_adt_or_prim,
            c.impl_blanket_param,
            c.impl_ref_collapsed,
            c.impl_tuple_fn_never,
            c.impl_marker_or_unstable_trait,
            c.trait_impl_assoc_tys_missing,
            c.trait_impl_assoc_fns_missing,
        );
        if !c.unstable_traits_sample.is_empty() {
            println!("  unstable_traits_sample: {}", c.unstable_traits_sample.join(", "));
        }
        if !c.unstable_ops_assoc_sample.is_empty() {
            println!("  unstable_ops_assoc_sample: {}", c.unstable_ops_assoc_sample.join(", "));
        }
        if !c.marker_or_empty_traits.is_empty() {
            println!("  marker_or_empty_traits ({}): {}", c.marker_or_empty_traits.len(), c.marker_or_empty_traits.join(", "));
        }
    }
    println!("=== TOP MODULE FAMILIES ===");
    for f in &rep.families {
        println!(
            "  {:26} pub_stable={:4}/{:<4} unstable={:<4} doc_hidden={:<4} foreign_reexp={:<3} fields_miss={:<4} marker_tr={:<2} family_trait_impls={:<4} family_assoc_tys={}",
            f.family,
            f.pub_stable_in_ridx,
            f.pub_stable_total,
            f.pub_unstable,
            f.doc_hidden,
            f.foreign_reexp,
            f.pub_fields_missing,
            f.marker_or_empty_traits,
            f.trait_impls_of_family_traits,
            f.trait_impl_assoc_tys_of_family_traits,
        );
    }
    println!("=== FOCUS SUBFAMILIES (ops / fmt / io / collections / iter) ===");
    for f in &rep.subfamilies {
        println!(
            "  {:34} pub_stable={:4}/{:<4} unstable={:<4} doc_hidden={:<4} foreign_reexp={:<3} fields_miss={:<4} marker_tr={:<2} family_trait_impls={:<4} family_assoc_tys={}",
            f.family,
            f.pub_stable_in_ridx,
            f.pub_stable_total,
            f.pub_unstable,
            f.doc_hidden,
            f.foreign_reexp,
            f.pub_fields_missing,
            f.marker_or_empty_traits,
            f.trait_impls_of_family_traits,
            f.trait_impl_assoc_tys_of_family_traits,
        );
    }
}
