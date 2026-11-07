/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Input descriptors and zero-copy views over `GenericParamEntry` and `WherePredEntry` tables.

use crate::format::{
    GenericOwner, GenericParamEntry, GenericParamKind, TyDefId, TyId, WherePredEntry, WherePredKind, GENERIC_PARAM_FLAG_HAS_DEFAULT, GENERIC_PARAM_FLAG_HOST_EFFECT,
    GENERIC_PARAM_FLAG_IS_SELF, GENERIC_PARAM_FLAG_SIZED, GENERIC_PARAM_FLAG_SYNTHETIC, WHERE_FLAG_CONST, WHERE_FLAG_HRTB, WHERE_FLAG_NEGATIVE,
};

use super::intern::RawGenericArg;
use super::iter::GenericArgIter;
use super::TyArenaView;

/// Writer input for a single generic parameter (`GenericParamEntry`).
#[derive(Debug, Clone, Copy)]
pub struct RawGenericParam<'a> {
    pub def_index: Option<u32>,
    pub name: &'a str,
    pub param_idx: u16,
    pub kind: GenericParamKind,
    pub ty_or_default: Option<TyId>,
    pub flags: u8,
}

/// Writer input for a single `where` / bound / supertrait / projection predicate (`WherePredEntry`).
#[derive(Debug, Clone, Copy)]
pub enum RawWherePred<'a> {
    TraitBound { lhs_ty: TyId, trait_def: TyDefId, trait_args: &'a [RawGenericArg<'a>], flags: u8 },
    ProjectionEq { self_ty: TyId, trait_def: TyDefId, trait_args: &'a [RawGenericArg<'a>], assoc_name: &'a str, rhs_ty: Option<TyId>, flags: u8 },
    TypeOutlives { lhs_ty: TyId, region: u32, flags: u8 },
    RegionOutlives { lhs_region: u32, rhs_region: u32, flags: u8 },
}

/// Zero-copy decoded view of a `GenericParamEntry`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenericParamView<'a> {
    pub owner: GenericOwner,
    pub def_index: Option<u32>,
    pub name: &'a str,
    pub param_idx: u16,
    pub kind: GenericParamKind,
    pub ty_or_default: Option<TyId>,
    pub flags: u8,
}

impl GenericParamView<'_> {
    #[inline]
    pub const fn has_default(&self) -> bool {
        (self.flags & GENERIC_PARAM_FLAG_HAS_DEFAULT) != 0
    }

    #[inline]
    pub const fn is_synthetic(&self) -> bool {
        (self.flags & GENERIC_PARAM_FLAG_SYNTHETIC) != 0
    }

    #[inline]
    pub const fn is_self(&self) -> bool {
        (self.flags & GENERIC_PARAM_FLAG_IS_SELF) != 0
    }

    #[inline]
    pub const fn is_sized(&self) -> bool {
        (self.flags & GENERIC_PARAM_FLAG_SIZED) != 0
    }

    #[inline]
    pub const fn is_host_effect(&self) -> bool {
        (self.flags & GENERIC_PARAM_FLAG_HOST_EFFECT) != 0
    }

    /// Default `TyId` for `Type` parameters that declare `= DefaultTy`.
    #[inline]
    pub const fn default_ty(&self) -> Option<TyId> {
        if matches!(self.kind, GenericParamKind::Type) && self.has_default() {
            self.ty_or_default
        } else {
            None
        }
    }

    /// Value `TyId` for `Const` parameters (`const N: Ty`).
    #[inline]
    pub const fn const_ty(&self) -> Option<TyId> {
        if matches!(self.kind, GenericParamKind::Const) {
            self.ty_or_default
        } else {
            None
        }
    }
}

/// Zero-allocation iterator over a contiguous slice of `GenericParamEntry` records.
#[derive(Debug, Clone, Copy)]
pub struct GenericParamIter<'a> {
    arena: TyArenaView<'a>,
    entries: &'a [GenericParamEntry],
}

impl<'a> GenericParamIter<'a> {
    #[inline]
    pub const fn new(arena: TyArenaView<'a>, entries: &'a [GenericParamEntry]) -> Self {
        Self { arena, entries }
    }

    #[inline]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[inline]
    fn decode(arena: TyArenaView<'a>, raw: GenericParamEntry) -> GenericParamView<'a> {
        let e = raw.from_le();
        GenericParamView {
            owner: GenericOwner::from_raw(e.owner_key),
            def_index: (e.def_index != u32::MAX).then_some(e.def_index),
            name: arena.str_at(e.name_offset),
            param_idx: e.param_idx,
            kind: GenericParamKind::from_u8(e.kind),
            ty_or_default: (e.ty_or_default != u32::MAX).then_some(TyId(e.ty_or_default)),
            flags: e.flags,
        }
    }

    #[inline]
    pub fn get(&self, idx: usize) -> Option<GenericParamView<'a>> {
        self.entries.get(idx).copied().map(|r| Self::decode(self.arena, r))
    }
}

impl<'a> Iterator for GenericParamIter<'a> {
    type Item = GenericParamView<'a>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let (&first, rest) = self.entries.split_first()?;
        self.entries = rest;
        Some(Self::decode(self.arena, first))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.entries.len(), Some(self.entries.len()))
    }
}

impl ExactSizeIterator for GenericParamIter<'_> {}

/// Zero-copy decoded view of a `WherePredEntry`.
#[derive(Debug, Clone, Copy)]
pub struct WherePredView<'a> {
    arena: TyArenaView<'a>,
    entry: WherePredEntry,
}

impl<'a> WherePredView<'a> {
    #[inline]
    pub fn new(arena: TyArenaView<'a>, raw: WherePredEntry) -> Self {
        Self { arena, entry: raw.from_le() }
    }

    #[inline]
    pub const fn owner(&self) -> GenericOwner {
        GenericOwner::from_raw(self.entry.owner_key)
    }

    #[inline]
    pub const fn kind(&self) -> WherePredKind {
        WherePredKind::from_u8(self.entry.kind)
    }

    #[inline]
    pub const fn is_hrtb(&self) -> bool {
        (self.entry.flags & WHERE_FLAG_HRTB) != 0
    }

    #[inline]
    pub const fn is_negative(&self) -> bool {
        (self.entry.flags & WHERE_FLAG_NEGATIVE) != 0
    }

    #[inline]
    pub const fn is_const(&self) -> bool {
        (self.entry.flags & WHERE_FLAG_CONST) != 0
    }

    #[inline]
    pub const fn lhs_ty(&self) -> TyId {
        TyId(self.entry.lhs_ty)
    }

    #[inline]
    pub const fn lhs_region(&self) -> u32 {
        self.entry.lhs_ty
    }

    #[inline]
    pub const fn trait_def(&self) -> Option<TyDefId> {
        if self.entry.trait_def == u32::MAX {
            None
        } else {
            Some(TyDefId(self.entry.trait_def))
        }
    }

    #[inline]
    pub fn trait_args(&self) -> GenericArgIter<'a> {
        GenericArgIter::from_words(self.arena.words_slice(self.entry.trait_args_start, u32::from(self.entry.trait_args_len)))
    }

    #[inline]
    pub fn assoc_name(&self) -> Option<&'a str> {
        (self.entry.assoc_name_offset != u32::MAX).then(|| self.arena.str_at(self.entry.assoc_name_offset))
    }

    #[inline]
    pub const fn rhs_ty(&self) -> Option<TyId> {
        if matches!(self.kind(), WherePredKind::ProjectionEq) && self.entry.rhs_or_region != u32::MAX {
            Some(TyId(self.entry.rhs_or_region))
        } else {
            None
        }
    }

    #[inline]
    pub const fn outlives_region(&self) -> Option<u32> {
        if matches!(self.kind(), WherePredKind::TypeOutlives | WherePredKind::RegionOutlives) {
            Some(self.entry.rhs_or_region)
        } else {
            None
        }
    }
}

/// Zero-allocation iterator over a contiguous slice of `WherePredEntry` records.
#[derive(Debug, Clone, Copy)]
pub struct WherePredIter<'a> {
    arena: TyArenaView<'a>,
    entries: &'a [WherePredEntry],
}

impl<'a> WherePredIter<'a> {
    #[inline]
    pub const fn new(arena: TyArenaView<'a>, entries: &'a [WherePredEntry]) -> Self {
        Self { arena, entries }
    }

    #[inline]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl<'a> Iterator for WherePredIter<'a> {
    type Item = WherePredView<'a>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let (&first, rest) = self.entries.split_first()?;
        self.entries = rest;
        Some(WherePredView::new(self.arena, first))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.entries.len(), Some(self.entries.len()))
    }
}

impl ExactSizeIterator for WherePredIter<'_> {}
