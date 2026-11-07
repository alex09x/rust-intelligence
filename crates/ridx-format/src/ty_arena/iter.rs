/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Zero-allocation iterators and decoded element views over `TyArena` packed slices.

use crate::format::{ConstArg, GenericArgWord, TyBoundEntry, TyDefId, TyId};

use super::TyArenaView;

/// Decoded generic argument from `ty_args`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenericArgView {
    Type(TyId),
    Const(ConstArg),
    Lifetime(u32),
}

impl GenericArgView {
    #[inline]
    pub const fn decode(raw: u32) -> Self {
        let w = GenericArgWord(raw);
        match w.tag_bits() {
            GenericArgWord::TAG_TYPE => Self::Type(TyId(w.payload())),
            GenericArgWord::TAG_LIFETIME => Self::Lifetime(w.payload()),
            _ => Self::Const(GenericArgWord::decode_const(raw)),
        }
    }
}

/// Zero-allocation iterator over `GenericArgWord`s stored in `ty_args`.
#[derive(Debug, Clone, Copy)]
pub struct GenericArgIter<'a> {
    pub(crate) words: &'a [u32],
}

impl<'a> GenericArgIter<'a> {
    pub const EMPTY: Self = Self { words: &[] };

    #[inline]
    pub const fn from_words(words: &'a [u32]) -> Self {
        Self { words }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.words.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    #[inline]
    pub fn get(&self, idx: usize) -> Option<GenericArgView> {
        self.words.get(idx).map(|&w| GenericArgView::decode(u32::from_le(w)))
    }
}

impl<'a> Iterator for GenericArgIter<'a> {
    type Item = GenericArgView;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let (&first, rest) = self.words.split_first()?;
        self.words = rest;
        Some(GenericArgView::decode(u32::from_le(first)))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.words.len(), Some(self.words.len()))
    }
}

impl ExactSizeIterator for GenericArgIter<'_> {}

/// Zero-allocation iterator over `TyId`s stored in `ty_args` (tuples and `fn` inputs).
#[derive(Debug, Clone, Copy)]
pub struct TyListIter<'a> {
    pub(crate) words: &'a [u32],
}

impl<'a> TyListIter<'a> {
    pub const EMPTY: Self = Self { words: &[] };

    #[inline]
    pub const fn from_words(words: &'a [u32]) -> Self {
        Self { words }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.words.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    #[inline]
    pub fn get(&self, idx: usize) -> Option<TyId> {
        self.words.get(idx).map(|&w| TyId(u32::from_le(w)))
    }
}

impl<'a> Iterator for TyListIter<'a> {
    type Item = TyId;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let (&first, rest) = self.words.split_first()?;
        self.words = rest;
        Some(TyId(u32::from_le(first)))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.words.len(), Some(self.words.len()))
    }
}

impl ExactSizeIterator for TyListIter<'_> {}

/// Zero-allocation iterator over raw `u32` indices stored in `ty_args` (e.g. `ImplEntry` assoc def indices).
#[derive(Debug, Clone, Copy)]
pub struct U32ListIter<'a> {
    words: &'a [u32],
}

impl<'a> U32ListIter<'a> {
    pub const EMPTY: Self = Self { words: &[] };

    #[inline]
    pub const fn from_words(words: &'a [u32]) -> Self {
        Self { words }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.words.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    #[inline]
    pub fn get(&self, idx: usize) -> Option<u32> {
        self.words.get(idx).map(|&w| u32::from_le(w))
    }
}

impl<'a> Iterator for U32ListIter<'a> {
    type Item = u32;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let (&first, rest) = self.words.split_first()?;
        self.words = rest;
        Some(u32::from_le(first))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.words.len(), Some(self.words.len()))
    }
}

impl ExactSizeIterator for U32ListIter<'_> {}

/// Zero-copy associated type equality binding (`Item = u8`) inside a `TyBoundView`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssocBindingView<'a> {
    pub name: &'a str,
    pub ty: TyId,
}

/// Zero-allocation iterator over `(assoc_name_off, assoc_ty_id)` pairs in `ty_args`.
#[derive(Debug, Clone, Copy)]
pub struct AssocBindingIter<'a> {
    pub(crate) arena: TyArenaView<'a>,
    pub(crate) words: &'a [u32],
}

impl<'a> Iterator for AssocBindingIter<'a> {
    type Item = AssocBindingView<'a>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.words.len() < 2 {
            self.words = &[];
            return None;
        }
        let name_off = u32::from_le(self.words[0]);
        let ty = TyId(u32::from_le(self.words[1]));
        self.words = &self.words[2..];
        Some(AssocBindingView { name: self.arena.str_at(name_off), ty })
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.words.len() / 2;
        (n, Some(n))
    }
}

impl ExactSizeIterator for AssocBindingIter<'_> {}

/// Zero-copy view of a single `TyBoundEntry`.
#[derive(Debug, Clone, Copy)]
pub struct TyBoundView<'a> {
    pub trait_def: TyDefId,
    pub is_maybe: bool,
    pub flags: u32,
    pub args: GenericArgIter<'a>,
    pub assocs: AssocBindingIter<'a>,
}

/// Zero-allocation iterator over a contiguous slice of `TyBoundEntry` records.
#[derive(Debug, Clone, Copy)]
pub struct TyBoundIter<'a> {
    pub(crate) arena: TyArenaView<'a>,
    pub(crate) bounds: &'a [TyBoundEntry],
}

impl<'a> TyBoundIter<'a> {
    #[inline]
    pub fn len(&self) -> usize {
        self.bounds.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.bounds.is_empty()
    }
}

impl<'a> Iterator for TyBoundIter<'a> {
    type Item = TyBoundView<'a>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let (&raw, rest) = self.bounds.split_first()?;
        self.bounds = rest;
        Some(self.arena.decode_bound(raw.from_le()))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.bounds.len(), Some(self.bounds.len()))
    }
}

impl ExactSizeIterator for TyBoundIter<'_> {}
