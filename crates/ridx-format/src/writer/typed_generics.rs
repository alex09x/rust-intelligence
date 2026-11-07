/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `IndexWriter` encoding helpers for `GenericParamEntry` and `WherePredEntry` (1.2-G4).

use crate::format::{GenericOwner, GenericParamEntry, WherePredEntry, WherePredKind};
use crate::ty_arena::{RawGenericParam, RawWherePred};

use super::IndexWriter;

impl IndexWriter {
    pub(super) fn encode_generic_param(&mut self, owner_key: u32, param: RawGenericParam<'_>) -> GenericParamEntry {
        let name_offset = self.intern_string(param.name);
        GenericParamEntry {
            owner_key,
            def_index: param.def_index.unwrap_or(u32::MAX),
            name_offset,
            ty_or_default: param.ty_or_default.map_or(u32::MAX, |t| t.0),
            param_idx: param.param_idx,
            kind: param.kind as u8,
            flags: param.flags,
        }
    }

    pub(super) fn encode_where_pred(&mut self, owner_key: u32, pred: RawWherePred<'_>) -> WherePredEntry {
        match pred {
            RawWherePred::TraitBound { lhs_ty, trait_def, trait_args, flags } => {
                let pool = &mut self.string_pool;
                let map = &mut self.string_map;
                let (trait_args_start, trait_args_len) = self.ty_interner.intern_generic_args_list(trait_args, |s| Self::intern_in_pool(pool, map, s));
                WherePredEntry {
                    owner_key,
                    lhs_ty: lhs_ty.0,
                    trait_def: trait_def.0,
                    trait_args_start,
                    rhs_or_region: u32::MAX,
                    assoc_name_offset: u32::MAX,
                    trait_args_len,
                    kind: WherePredKind::TraitBound as u8,
                    flags,
                }
            }
            RawWherePred::ProjectionEq { self_ty, trait_def, trait_args, assoc_name, rhs_ty, flags } => {
                let pool = &mut self.string_pool;
                let map = &mut self.string_map;
                let (trait_args_start, trait_args_len) = self.ty_interner.intern_generic_args_list(trait_args, |s| Self::intern_in_pool(pool, map, s));
                let assoc_name_offset = self.intern_string(assoc_name);
                WherePredEntry {
                    owner_key,
                    lhs_ty: self_ty.0,
                    trait_def: trait_def.0,
                    trait_args_start,
                    rhs_or_region: rhs_ty.map_or(u32::MAX, |t| t.0),
                    assoc_name_offset,
                    trait_args_len,
                    kind: WherePredKind::ProjectionEq as u8,
                    flags,
                }
            }
            RawWherePred::TypeOutlives { lhs_ty, region, flags } => WherePredEntry {
                owner_key,
                lhs_ty: lhs_ty.0,
                trait_def: u32::MAX,
                trait_args_start: 0,
                rhs_or_region: region,
                assoc_name_offset: u32::MAX,
                trait_args_len: 0,
                kind: WherePredKind::TypeOutlives as u8,
                flags,
            },
            RawWherePred::RegionOutlives { lhs_region, rhs_region, flags } => WherePredEntry {
                owner_key,
                lhs_ty: lhs_region,
                trait_def: u32::MAX,
                trait_args_start: 0,
                rhs_or_region: rhs_region,
                assoc_name_offset: u32::MAX,
                trait_args_len: 0,
                kind: WherePredKind::RegionOutlives as u8,
                flags,
            },
        }
    }

    /// Records a generic parameter declared on `DefEntry` `def_index`.
    pub fn add_def_generic_param(&mut self, def_index: u32, param: RawGenericParam<'_>) {
        let entry = self.encode_generic_param(GenericOwner::Def(def_index).to_raw(), param);
        self.def_generic_params.push(entry);
    }

    /// Records a generic parameter declared on the queued `Impl` at `pending_impl_idx` (returned by `add_impl`).
    pub fn add_impl_generic_param(&mut self, pending_impl_idx: u32, param: RawGenericParam<'_>) {
        let entry = self.encode_generic_param(0, param);
        if let Some(imp) = self.impls.get_mut(pending_impl_idx as usize) {
            imp.generic_params.push(entry);
        }
    }

    /// Records a `where` / bound / supertrait / item-bound predicate on `DefEntry` `def_index`.
    pub fn add_def_where_pred(&mut self, def_index: u32, pred: RawWherePred<'_>) {
        let entry = self.encode_where_pred(GenericOwner::Def(def_index).to_raw(), pred);
        self.def_where_preds.push(entry);
    }

    /// Records a `where` / bound predicate on the queued `Impl` at `pending_impl_idx` (returned by `add_impl`).
    pub fn add_impl_where_pred(&mut self, pending_impl_idx: u32, pred: RawWherePred<'_>) {
        let entry = self.encode_where_pred(0, pred);
        if let Some(imp) = self.impls.get_mut(pending_impl_idx as usize) {
            imp.where_preds.push(entry);
        }
    }

    /// Total count of generic parameter declarations queued across definitions and impls.
    pub fn generic_param_count(&self) -> usize {
        self.def_generic_params.len() + self.impls.iter().map(|i| i.generic_params.len()).sum::<usize>()
    }

    /// Total count of `where` / bound predicates queued across definitions and impls.
    pub fn where_pred_count(&self) -> usize {
        self.def_where_preds.len() + self.impls.iter().map(|i| i.where_preds.len()).sum::<usize>()
    }
}
