/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `IndexWriter` encoding helpers for `GenericParamEntry`, `WherePredEntry`, `LangItemEntry`, and `UseEntry`.

use crate::format::{GenericOwner, GenericParamEntry, LangItemEntry, UseEntry, UsePrimTy, WherePredEntry, WherePredKind, USE_FLAG_GLOB, USE_FLAG_PRIM_TY, USE_FLAG_UNDERSCORE};
use crate::ty_arena::{RawGenericParam, RawLangItem, RawUse, RawWherePred};

use super::IndexWriter;

#[inline]
pub(crate) const fn use_kind_bucket(flags: u8) -> u8 {
    if (flags & USE_FLAG_GLOB) != 0 {
        1
    } else if (flags & USE_FLAG_UNDERSCORE) != 0 {
        2
    } else {
        0
    }
}

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

    /// Records a compiler `#[lang = "..."]` (or associated trait item) entry in this shard.
    pub fn add_lang_item(&mut self, item: RawLangItem) {
        self.lang_items.push(LangItemEntry { kind: item.kind as u16, _pad: 0, ty_def: item.ty_def.0, def_index: item.def_index.unwrap_or(u32::MAX) });
    }

    /// Total count of `LangItemEntry` rows queued in this shard.
    pub fn lang_item_count(&self) -> usize {
        self.lang_items.len()
    }

    /// Records a module `use`, glob, re-export, or `extern crate` binding in this shard.
    pub fn add_use(&mut self, item: RawUse<'_>) {
        let name_offset = self.intern_string(item.name);
        let mut flags = item.flags;
        if item.prim_ty != UsePrimTy::None {
            flags |= USE_FLAG_PRIM_TY;
        }
        self.uses.push(UseEntry {
            owner_raw: item.owner.to_raw(),
            def_index: item.def_index.unwrap_or(u32::MAX),
            name_offset,
            type_def: item.type_def.map_or(u32::MAX, |d| d.0),
            value_def: item.value_def.map_or(u32::MAX, |d| d.0),
            macro_def: item.macro_def.map_or(u32::MAX, |d| d.0),
            vis: item.vis as u8,
            flags,
            prim_tag: item.prim_ty as u8,
            _pad: 0,
        });
    }

    /// Total count of raw `UseEntry` rows queued before multi-namespace merge.
    pub fn use_count(&self) -> usize {
        self.uses.len()
    }

    pub(super) fn prepare_sorted_uses(&self) -> Vec<UseEntry> {
        let mut list = self.uses.clone();
        list.sort_by(|a, b| {
            (a.owner_raw, use_kind_bucket(a.flags), self.get_interned_str(a.name_offset), a.vis, a.def_index, a.type_def, a.value_def, a.macro_def, a.prim_tag).cmp(&(
                b.owner_raw,
                use_kind_bucket(b.flags),
                self.get_interned_str(b.name_offset),
                b.vis,
                b.def_index,
                b.type_def,
                b.value_def,
                b.macro_def,
                b.prim_tag,
            ))
        });
        let mut merged: Vec<UseEntry> = Vec::with_capacity(list.len());
        for entry in list {
            if let Some(prev) = merged.last_mut() {
                let bucket = use_kind_bucket(entry.flags);
                let prev_bucket = use_kind_bucket(prev.flags);
                let same_named = bucket == 0 && prev_bucket == 0 && prev.owner_raw == entry.owner_raw && prev.name_offset == entry.name_offset;
                let same_special = bucket != 0
                    && bucket == prev_bucket
                    && prev.owner_raw == entry.owner_raw
                    && prev.name_offset == entry.name_offset
                    && prev.type_def == entry.type_def
                    && prev.value_def == entry.value_def
                    && prev.macro_def == entry.macro_def
                    && prev.prim_tag == entry.prim_tag;
                if same_named || same_special {
                    if prev.def_index == u32::MAX {
                        prev.def_index = entry.def_index;
                    } else if entry.def_index != u32::MAX {
                        prev.def_index = prev.def_index.min(entry.def_index);
                    }
                    if prev.type_def == u32::MAX {
                        prev.type_def = entry.type_def;
                    }
                    if prev.value_def == u32::MAX {
                        prev.value_def = entry.value_def;
                    }
                    if prev.macro_def == u32::MAX {
                        prev.macro_def = entry.macro_def;
                    }
                    prev.vis = prev.vis.min(entry.vis);
                    prev.flags |= entry.flags;
                    if prev.prim_tag == 0 {
                        prev.prim_tag = entry.prim_tag;
                    }
                    continue;
                }
            }
            merged.push(entry);
        }
        merged
    }
}
