/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Writer input and zero-copy decoded view/iterator over `UseEntry` tables (1.2-G7).

use crate::format::{
    ItemVis, TyDefId, UseEntry, UseOwner, UsePrimTy, USE_FLAG_EXTERN_CRATE, USE_FLAG_GLOB, USE_FLAG_PRELUDE, USE_FLAG_PRIM_TY, USE_FLAG_REEXPORT, USE_FLAG_UNDERSCORE,
};

use super::TyArenaView;

/// Writer input for a module `use`, glob, re-export, or `extern crate` binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawUse<'a> {
    pub owner: UseOwner,
    pub def_index: Option<u32>,
    pub name: &'a str,
    pub type_def: Option<TyDefId>,
    pub value_def: Option<TyDefId>,
    pub macro_def: Option<TyDefId>,
    pub vis: ItemVis,
    pub flags: u8,
    pub prim_ty: UsePrimTy,
}

/// Zero-copy decoded view of a `UseEntry`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UseView<'a> {
    pub owner: UseOwner,
    pub def_index: Option<u32>,
    pub name: &'a str,
    pub type_def: Option<TyDefId>,
    pub value_def: Option<TyDefId>,
    pub macro_def: Option<TyDefId>,
    pub vis: ItemVis,
    pub flags: u8,
    pub prim_ty: UsePrimTy,
}

impl<'a> UseView<'a> {
    #[inline]
    pub fn from_raw(arena: TyArenaView<'a>, raw: UseEntry) -> Self {
        Self {
            owner: raw.owner(),
            def_index: raw.def_index(),
            name: arena.str_at(raw.name_offset()),
            type_def: raw.type_def(),
            value_def: raw.value_def(),
            macro_def: raw.macro_def(),
            vis: raw.visibility(),
            flags: raw.flags,
            prim_ty: raw.prim_ty(),
        }
    }

    #[inline]
    pub const fn is_glob(&self) -> bool {
        (self.flags & USE_FLAG_GLOB) != 0
    }

    #[inline]
    pub const fn is_reexport(&self) -> bool {
        (self.flags & USE_FLAG_REEXPORT) != 0
    }

    #[inline]
    pub const fn is_extern_crate(&self) -> bool {
        (self.flags & USE_FLAG_EXTERN_CRATE) != 0
    }

    #[inline]
    pub const fn is_underscore(&self) -> bool {
        (self.flags & USE_FLAG_UNDERSCORE) != 0
    }

    #[inline]
    pub const fn has_prim_ty(&self) -> bool {
        (self.flags & USE_FLAG_PRIM_TY) != 0
    }

    #[inline]
    pub const fn is_prelude(&self) -> bool {
        (self.flags & USE_FLAG_PRELUDE) != 0
    }
}

/// Zero-allocation iterator over `UseEntry` records.
#[derive(Debug, Clone, Copy)]
pub struct UseIter<'a> {
    arena: TyArenaView<'a>,
    entries: &'a [UseEntry],
}

impl<'a> UseIter<'a> {
    #[inline]
    pub const fn new(arena: TyArenaView<'a>, entries: &'a [UseEntry]) -> Self {
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

impl<'a> Iterator for UseIter<'a> {
    type Item = UseView<'a>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let (&first, rest) = self.entries.split_first()?;
        self.entries = rest;
        Some(UseView::from_raw(self.arena, first))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.entries.len(), Some(self.entries.len()))
    }
}

impl ExactSizeIterator for UseIter<'_> {}
