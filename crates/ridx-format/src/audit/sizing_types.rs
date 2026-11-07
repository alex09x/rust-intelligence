/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Binary layout constants and report structs for Phase 1 gap-table sizing.

use serde::Serialize;

pub const BYTES_TY_NODE: u64 = 12;
pub const BYTES_TY_ARG: u64 = 4;
pub const BYTES_FN_SIG: u64 = 24;
pub const BYTES_FN_PARAM: u64 = 8;
pub const BYTES_FIELD_OR_VAL_TY: u64 = 12;
pub const BYTES_GENERIC_PARAM: u64 = 12;
pub const BYTES_WHERE_PRED: u64 = 16;
pub const BYTES_IMPL_HEADER: u64 = 28;
pub const BYTES_IMPL_ASSOC_REF: u64 = 4;
pub const BYTES_ASSOC_TY_BINDING: u64 = 12;
pub const BYTES_TRAIT_IMPL_INDEX: u64 = 12;
pub const BYTES_LANG_ITEM_ROW: u64 = 32;
pub const BYTES_USE_TARGET: u64 = 16;
pub const BYTES_CRATE_CFG_META: u64 = 64;
pub const BYTES_MACRO_DEF_HDR: u64 = 16;
pub const BYTES_MACRO_CALLSITE_HDR: u64 = 32;

/// Measured v4 byte breakdown for a shard tier (`ws`, `deps`, `sysroot`, or `total`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct TierSectionBytes {
    pub shards: usize,
    pub disk_bytes: u64,
    pub string_pool_bytes: u64,
    pub unique_sig_string_bytes: u64,
    pub item_tables_bytes: u64,
    pub body_refs_bytes: u64,
    pub body_types_notes_bytes: u64,
}

impl TierSectionBytes {
    pub fn add_assign(&mut self, rhs: &Self) {
        self.shards += rhs.shards;
        self.disk_bytes += rhs.disk_bytes;
        self.string_pool_bytes += rhs.string_pool_bytes;
        self.unique_sig_string_bytes += rhs.unique_sig_string_bytes;
        self.item_tables_bytes += rhs.item_tables_bytes;
        self.body_refs_bytes += rhs.body_refs_bytes;
        self.body_types_notes_bytes += rhs.body_types_notes_bytes;
    }
}

/// Exact counts of lang-item trait implementations currently present in `derefs`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct LangItemCounts {
    pub deref_targets: u64,
    pub deref_mut: u64,
    pub index_and_mut: u64,
    pub into_iterator_and_iter: u64,
    pub try_trait: u64,
    pub future_trait: u64,
}

impl LangItemCounts {
    pub fn total(&self) -> u64 {
        self.deref_targets + self.deref_mut + self.index_and_mut + self.into_iterator_and_iter + self.try_trait + self.future_trait
    }

    pub fn add_assign(&mut self, rhs: &Self) {
        self.deref_targets += rhs.deref_targets;
        self.deref_mut += rhs.deref_mut;
        self.index_and_mut += rhs.index_and_mut;
        self.into_iterator_and_iter += rhs.into_iterator_and_iter;
        self.try_trait += rhs.try_trait;
        self.future_trait += rhs.future_trait;
    }
}

/// Empirical byte projection for every Phase 1 gap table in a shard tier.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct GapTableBytes {
    pub g1_ty_arena_bytes: u64,
    pub g2_fn_sigs_bytes: u64,
    pub g3_fields_variants_vals_bytes: u64,
    pub g4_generics_bounds_bytes: u64,
    pub g5_impls_assoc_bytes: u64,
    pub g6_lang_items_bytes: u64,
    pub g7_reexports_vis_cfg_bytes: u64,
    pub g8_macros_bytes: u64,
    pub unique_ty_nodes: u64,
    pub fn_count: u64,
    pub fn_params: u64,
    pub typed_fields_and_vals: u64,
    pub generic_params: u64,
    pub where_preds: u64,
    pub impl_blocks: u64,
    pub macro_defs: u64,
    pub ws_macro_callsites: u64,
}

impl GapTableBytes {
    pub fn total_additive_bytes(&self) -> u64 {
        self.g1_ty_arena_bytes
            + self.g2_fn_sigs_bytes
            + self.g3_fields_variants_vals_bytes
            + self.g4_generics_bounds_bytes
            + self.g5_impls_assoc_bytes
            + self.g6_lang_items_bytes
            + self.g7_reexports_vis_cfg_bytes
            + self.g8_macros_bytes
    }

    pub fn add_assign(&mut self, rhs: &Self) {
        self.g1_ty_arena_bytes += rhs.g1_ty_arena_bytes;
        self.g2_fn_sigs_bytes += rhs.g2_fn_sigs_bytes;
        self.g3_fields_variants_vals_bytes += rhs.g3_fields_variants_vals_bytes;
        self.g4_generics_bounds_bytes += rhs.g4_generics_bounds_bytes;
        self.g5_impls_assoc_bytes += rhs.g5_impls_assoc_bytes;
        self.g6_lang_items_bytes += rhs.g6_lang_items_bytes;
        self.g7_reexports_vis_cfg_bytes += rhs.g7_reexports_vis_cfg_bytes;
        self.g8_macros_bytes += rhs.g8_macros_bytes;
        self.unique_ty_nodes += rhs.unique_ty_nodes;
        self.fn_count += rhs.fn_count;
        self.fn_params += rhs.fn_params;
        self.typed_fields_and_vals += rhs.typed_fields_and_vals;
        self.generic_params += rhs.generic_params;
        self.where_preds += rhs.where_preds;
        self.impl_blocks += rhs.impl_blocks;
        self.macro_defs += rhs.macro_defs;
        self.ws_macro_callsites += rhs.ws_macro_callsites;
    }
}

/// Complete sizing audit for a workspace (split into `ws`, `deps`, `sysroot`, `total`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SizingAuditReport {
    pub ws_sections: TierSectionBytes,
    pub dep_sections: TierSectionBytes,
    pub sysroot_sections: TierSectionBytes,
    pub total_sections: TierSectionBytes,
    pub ws_gaps: GapTableBytes,
    pub dep_gaps: GapTableBytes,
    pub sysroot_gaps: GapTableBytes,
    pub total_gaps: GapTableBytes,
    pub lang_items: LangItemCounts,
}
