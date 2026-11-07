/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Input descriptors and zero-copy views over `ImplEntry` and `AssocTyEntry` tables.

use crate::format::{AssocTyEntry, ImplEntry, SimplifiedSelfKind, TyDefId, TyId, IMPL_FLAG_CONST, IMPL_FLAG_NEGATIVE, IMPL_FLAG_RESERVATION, IMPL_FLAG_TRAIT, IMPL_FLAG_UNSAFE};

use super::intern::RawGenericArg;
use super::iter::{GenericArgIter, U32ListIter};
use super::TyArenaView;

/// Writer input for an associated type binding inside an `impl` (`type Item = T;`).
#[derive(Debug, Clone, Copy)]
pub struct RawAssocTy<'a> {
    pub def_index: Option<u32>,
    pub name: &'a str,
    pub ty: TyId,
}

/// Writer input for an inherent or trait `impl` header.
#[derive(Debug, Clone, Copy)]
pub struct RawImpl<'a> {
    pub def_index: Option<u32>,
    pub self_ty: TyId,
    pub trait_def: Option<TyDefId>,
    pub trait_args: &'a [RawGenericArg<'a>],
    pub assoc_tys: &'a [RawAssocTy<'a>],
    pub assoc_defs: &'a [u32],
    pub flags: u8,
}

/// Zero-copy view of a single `AssocTyEntry`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssocTyView<'a> {
    pub impl_index: u32,
    pub def_index: Option<u32>,
    pub name: &'a str,
    pub ty: TyId,
}

/// Zero-allocation iterator over a contiguous slice of `AssocTyEntry` records.
#[derive(Debug, Clone, Copy)]
pub struct AssocTyIter<'a> {
    arena: TyArenaView<'a>,
    entries: &'a [AssocTyEntry],
}

impl<'a> AssocTyIter<'a> {
    #[inline]
    pub const fn new(arena: TyArenaView<'a>, entries: &'a [AssocTyEntry]) -> Self {
        Self { arena, entries }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl<'a> Iterator for AssocTyIter<'a> {
    type Item = AssocTyView<'a>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let (&raw, rest) = self.entries.split_first()?;
        self.entries = rest;
        let e = raw.from_le();
        Some(AssocTyView { impl_index: e.impl_index, def_index: (e.def_index != u32::MAX).then_some(e.def_index), name: self.arena.str_at(e.name_offset), ty: TyId(e.ty) })
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.entries.len(), Some(self.entries.len()))
    }
}

impl ExactSizeIterator for AssocTyIter<'_> {}

/// Zero-copy view of a decoded `ImplEntry` paired with its shard `TyArena` and `assoc_tys` table.
#[derive(Debug, Clone, Copy)]
pub struct ImplView<'a> {
    index: u32,
    entry: ImplEntry,
    arena: TyArenaView<'a>,
    all_assoc_tys: &'a [AssocTyEntry],
}

impl<'a> ImplView<'a> {
    #[inline]
    pub fn new(index: u32, raw: ImplEntry, arena: TyArenaView<'a>, all_assoc_tys: &'a [AssocTyEntry]) -> Self {
        Self { index, entry: raw.from_le(), arena, all_assoc_tys }
    }

    #[inline]
    pub const fn index(&self) -> u32 {
        self.index
    }

    #[inline]
    pub const fn def_index(&self) -> Option<u32> {
        if self.entry.def_index == u32::MAX {
            None
        } else {
            Some(self.entry.def_index)
        }
    }

    #[inline]
    pub const fn self_ty(&self) -> TyId {
        TyId(self.entry.self_ty)
    }

    #[inline]
    pub const fn simplified_self(&self) -> SimplifiedSelfKind {
        SimplifiedSelfKind::from_u8(self.entry.simplified_self)
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
    pub fn assoc_tys(&self) -> AssocTyIter<'a> {
        let s = self.entry.assoc_tys_start as usize;
        let end = s.saturating_add(self.entry.assoc_tys_len as usize);
        AssocTyIter::new(self.arena, self.all_assoc_tys.get(s..end).unwrap_or(&[]))
    }

    /// Finds the RHS `TyId` of an associated type binding by name (e.g. `"Item"`, `"Target"`).
    #[inline]
    pub fn find_assoc_ty(&self, name: &str) -> Option<TyId> {
        self.assoc_tys().find_map(|a| (a.name == name).then_some(a.ty))
    }

    #[inline]
    pub fn assoc_def_indices(&self) -> U32ListIter<'a> {
        U32ListIter::from_words(self.arena.words_slice(self.entry.assoc_defs_start, u32::from(self.entry.assoc_defs_len)))
    }

    #[inline]
    pub const fn flags(&self) -> u8 {
        self.entry.flags
    }

    #[inline]
    pub const fn is_trait(&self) -> bool {
        (self.entry.flags & IMPL_FLAG_TRAIT) != 0
    }

    #[inline]
    pub const fn is_inherent(&self) -> bool {
        !self.is_trait()
    }

    #[inline]
    pub const fn is_negative(&self) -> bool {
        (self.entry.flags & IMPL_FLAG_NEGATIVE) != 0
    }

    #[inline]
    pub const fn is_unsafe(&self) -> bool {
        (self.entry.flags & IMPL_FLAG_UNSAFE) != 0
    }

    #[inline]
    pub const fn is_reservation(&self) -> bool {
        (self.entry.flags & IMPL_FLAG_RESERVATION) != 0
    }

    #[inline]
    pub const fn is_const(&self) -> bool {
        (self.entry.flags & IMPL_FLAG_CONST) != 0
    }
}

/// Zero-allocation iterator over a slice of `ImplEntry` records.
#[derive(Debug, Clone, Copy)]
pub struct ImplIter<'a> {
    base_index: u32,
    entries: &'a [ImplEntry],
    arena: TyArenaView<'a>,
    all_assoc_tys: &'a [AssocTyEntry],
}

impl<'a> ImplIter<'a> {
    #[inline]
    pub const fn new(base_index: u32, entries: &'a [ImplEntry], arena: TyArenaView<'a>, all_assoc_tys: &'a [AssocTyEntry]) -> Self {
        Self { base_index, entries, arena, all_assoc_tys }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl<'a> Iterator for ImplIter<'a> {
    type Item = ImplView<'a>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let (&raw, rest) = self.entries.split_first()?;
        let idx = self.base_index;
        self.base_index = self.base_index.saturating_add(1);
        self.entries = rest;
        Some(ImplView::new(idx, raw, self.arena, self.all_assoc_tys))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.entries.len(), Some(self.entries.len()))
    }
}

impl ExactSizeIterator for ImplIter<'_> {}
