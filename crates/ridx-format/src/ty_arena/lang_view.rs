/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Writer input and zero-copy decoded view/iterator over `LangItemEntry` tables (1.2-G6).

use crate::format::{LangItemEntry, LangItemKind, TyDefId};

/// Writer input for a single `LangItemEntry`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawLangItem {
    pub kind: LangItemKind,
    pub ty_def: TyDefId,
    pub def_index: Option<u32>,
}

/// Zero-copy decoded view of a `LangItemEntry`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LangItemView {
    pub kind: LangItemKind,
    pub ty_def: TyDefId,
    pub def_index: Option<u32>,
}

impl LangItemView {
    #[inline]
    pub const fn from_raw(raw: LangItemEntry) -> Self {
        Self { kind: raw.lang_kind(), ty_def: raw.ty_def_id(), def_index: raw.def_index() }
    }
}

/// Zero-allocation iterator over sorted `LangItemEntry` records.
#[derive(Debug, Clone, Copy)]
pub struct LangItemIter<'a> {
    entries: &'a [LangItemEntry],
}

impl<'a> LangItemIter<'a> {
    #[inline]
    pub const fn new(entries: &'a [LangItemEntry]) -> Self {
        Self { entries }
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

impl Iterator for LangItemIter<'_> {
    type Item = LangItemView;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let (&first, rest) = self.entries.split_first()?;
        self.entries = rest;
        Some(LangItemView::from_raw(first))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.entries.len(), Some(self.entries.len()))
    }
}

impl ExactSizeIterator for LangItemIter<'_> {}
