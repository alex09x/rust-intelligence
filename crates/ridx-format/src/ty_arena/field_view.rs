/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Input descriptors and zero-copy views over `FieldEntry` and `DefTyEntry` tables.

use crate::format::{
    CtorShape, DefTyEntry, FieldEntry, ItemVis, TyId, DEF_TY_FLAG_FUNDAMENTAL, DEF_TY_FLAG_MUT_STATIC, DEF_TY_FLAG_NON_EXHAUSTIVE, DEF_TY_FLAG_UNSAFE_STATIC, FIELD_FLAG_TUPLE,
    FIELD_FLAG_UNSAFE,
};

use super::TyArenaView;

/// Writer input for an ADT or enum variant field (`FieldEntry`).
#[derive(Debug, Clone, Copy)]
pub struct RawField<'a> {
    pub owner_def_index: u32,
    pub def_index: Option<u32>,
    pub name: &'a str,
    pub ty: TyId,
    pub field_idx: u16,
    pub vis: ItemVis,
    pub flags: u8,
}

/// Writer input for a definition's RHS/value/ADT type and constructor shape (`DefTyEntry`).
#[derive(Debug, Clone, Copy)]
pub struct RawDefTy {
    pub def_index: u32,
    pub ty: TyId,
    pub parent_def_index: Option<u32>,
    pub variant_idx: Option<u16>,
    pub ctor_shape: CtorShape,
    pub flags: u8,
}

/// Zero-copy decoded view of a single `FieldEntry`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldView<'a> {
    pub owner_def_index: u32,
    pub def_index: Option<u32>,
    pub name: &'a str,
    pub ty: TyId,
    pub field_idx: u16,
    pub vis: ItemVis,
    pub flags: u8,
}

impl<'a> FieldView<'a> {
    #[inline]
    pub const fn is_tuple_field(&self) -> bool {
        (self.flags & FIELD_FLAG_TUPLE) != 0
    }

    #[inline]
    pub const fn is_unsafe(&self) -> bool {
        (self.flags & FIELD_FLAG_UNSAFE) != 0
    }
}

/// Zero-allocation iterator over a contiguous slice of `FieldEntry` records.
#[derive(Debug, Clone, Copy)]
pub struct FieldIter<'a> {
    arena: TyArenaView<'a>,
    entries: &'a [FieldEntry],
}

impl<'a> FieldIter<'a> {
    #[inline]
    pub const fn new(arena: TyArenaView<'a>, entries: &'a [FieldEntry]) -> Self {
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
    pub fn get(&self, idx: usize) -> Option<FieldView<'a>> {
        let raw = self.entries.get(idx)?.from_le();
        Some(FieldView {
            owner_def_index: raw.owner_def_index,
            def_index: if raw.def_index == u32::MAX { None } else { Some(raw.def_index) },
            name: self.arena.str_at(raw.name_offset),
            ty: TyId(raw.ty),
            field_idx: raw.field_idx,
            vis: ItemVis::from_u8(raw.vis),
            flags: raw.flags,
        })
    }
}

impl<'a> Iterator for FieldIter<'a> {
    type Item = FieldView<'a>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let (first, rest) = self.entries.split_first()?;
        self.entries = rest;
        let raw = first.from_le();
        Some(FieldView {
            owner_def_index: raw.owner_def_index,
            def_index: if raw.def_index == u32::MAX { None } else { Some(raw.def_index) },
            name: self.arena.str_at(raw.name_offset),
            ty: TyId(raw.ty),
            field_idx: raw.field_idx,
            vis: ItemVis::from_u8(raw.vis),
            flags: raw.flags,
        })
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.entries.len(), Some(self.entries.len()))
    }
}

impl<'a> ExactSizeIterator for FieldIter<'a> {}

/// Decoded view of a `DefTyEntry` row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefTyView {
    entry: DefTyEntry,
}

impl DefTyView {
    #[inline]
    pub fn new(raw: DefTyEntry) -> Self {
        Self { entry: raw.from_le() }
    }

    #[inline]
    pub const fn def_index(&self) -> u32 {
        self.entry.def_index
    }

    #[inline]
    pub const fn ty(&self) -> TyId {
        TyId(self.entry.ty)
    }

    #[inline]
    pub const fn parent_def_index(&self) -> Option<u32> {
        if self.entry.parent_def_index == u32::MAX {
            None
        } else {
            Some(self.entry.parent_def_index)
        }
    }

    #[inline]
    pub const fn variant_idx(&self) -> Option<u16> {
        if self.entry.variant_idx == u16::MAX {
            None
        } else {
            Some(self.entry.variant_idx)
        }
    }

    #[inline]
    pub const fn ctor_shape(&self) -> CtorShape {
        CtorShape::from_u8(self.entry.ctor_shape)
    }

    #[inline]
    pub const fn flags(&self) -> u8 {
        self.entry.flags
    }

    #[inline]
    pub const fn is_non_exhaustive(&self) -> bool {
        (self.entry.flags & DEF_TY_FLAG_NON_EXHAUSTIVE) != 0
    }

    #[inline]
    pub const fn is_mut_static(&self) -> bool {
        (self.entry.flags & DEF_TY_FLAG_MUT_STATIC) != 0
    }

    #[inline]
    pub const fn is_unsafe_static(&self) -> bool {
        (self.entry.flags & DEF_TY_FLAG_UNSAFE_STATIC) != 0
    }

    #[inline]
    pub const fn is_fundamental(&self) -> bool {
        (self.entry.flags & DEF_TY_FLAG_FUNDAMENTAL) != 0
    }
}
