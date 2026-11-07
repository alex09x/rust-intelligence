/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Zero-copy reader views, structural interner, and formatter for `TyArena`.

mod display;
mod fn_sig;
mod intern;

pub use self::fn_sig::{FnSigView, RawFnSig};
pub use self::intern::{RawGenericArg, RawTy, RawTyBound, RawTyDef, TyInterner};

use crate::format::{
    ConstArg, FloatTyWidth, FnAbiKind, GenericArgWord, IntTyWidth, TyBoundEntry, TyDefEntry, TyDefId, TyEntry, TyId, TyMutability, TyTag, UintTyWidth, BOUND_FLAG_MAYBE,
    FN_FLAG_UNSAFE, FN_FLAG_VARIADIC,
};

/// Resolved zero-copy view of a `TyDefEntry`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TyDefView<'a> {
    pub path_hash: u64,
    pub path: &'a str,
    pub local_def_idx: Option<u32>,
    pub extern_unit: Option<&'a str>,
}

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
    words: &'a [u32],
}

impl<'a> GenericArgIter<'a> {
    pub const EMPTY: Self = Self { words: &[] };

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
    words: &'a [u32],
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

/// Zero-copy associated type equality binding (`Item = u8`) inside a `TyBoundView`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssocBindingView<'a> {
    pub name: &'a str,
    pub ty: TyId,
}

/// Zero-allocation iterator over `(assoc_name_off, assoc_ty_id)` pairs in `ty_args`.
#[derive(Debug, Clone, Copy)]
pub struct AssocBindingIter<'a> {
    arena: TyArenaView<'a>,
    words: &'a [u32],
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
    arena: TyArenaView<'a>,
    bounds: &'a [TyBoundEntry],
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

/// Structured, `Infer`-free zero-copy decoded view of a `TyEntry`.
#[derive(Debug, Clone, Copy)]
pub enum TyKindView<'a> {
    Unit,
    Bool,
    Char,
    Int(IntTyWidth),
    Uint(UintTyWidth),
    Float(FloatTyWidth),
    Str,
    Never,
    Adt { def: TyDefId, args: GenericArgIter<'a> },
    Ref { mutability: TyMutability, inner: TyId, lifetime: u32 },
    Ptr { mutability: TyMutability, inner: TyId },
    Slice(TyId),
    Array { elem: TyId, len: ConstArg },
    Tuple(TyListIter<'a>),
    FnPtr { inputs: TyListIter<'a>, output: TyId, abi: FnAbiKind, is_unsafe: bool, is_variadic: bool },
    FnDef { def: TyDefId, args: GenericArgIter<'a> },
    Param { index: u32, name: &'a str },
    Assoc { trait_def: TyDefId, name: &'a str, self_ty: TyId, trait_args: GenericArgIter<'a> },
    Dyn { bounds: TyBoundIter<'a>, lifetime: u32 },
    ImplTrait { bounds: TyBoundIter<'a> },
    Unknown,
}

/// Zero-copy view over a shard's `TyArena` sections and string pool.
#[derive(Debug, Clone, Copy)]
pub struct TyArenaView<'a> {
    string_pool: &'a [u8],
    defs: &'a [TyDefEntry],
    entries: &'a [TyEntry],
    bounds: &'a [TyBoundEntry],
    args: &'a [u32],
}

impl<'a> TyArenaView<'a> {
    #[inline]
    pub const fn new(string_pool: &'a [u8], defs: &'a [TyDefEntry], entries: &'a [TyEntry], bounds: &'a [TyBoundEntry], args: &'a [u32]) -> Self {
        Self { string_pool, defs, entries, bounds, args }
    }

    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[inline]
    pub const fn ty_count(&self) -> usize {
        self.entries.len()
    }

    #[inline]
    pub const fn def_count(&self) -> usize {
        self.defs.len()
    }

    #[inline]
    pub const fn bound_count(&self) -> usize {
        self.bounds.len()
    }

    #[inline]
    pub const fn arg_count(&self) -> usize {
        self.args.len()
    }

    #[inline]
    pub fn str_at(&self, offset: u32) -> &'a str {
        let start = offset as usize;
        if start >= self.string_pool.len() {
            return "";
        }
        let end = self.string_pool[start..].iter().position(|&b| b == 0).map_or(self.string_pool.len(), |p| start + p);
        std::str::from_utf8(&self.string_pool[start..end]).unwrap_or("")
    }

    #[inline]
    pub fn def(&self, id: TyDefId) -> Option<TyDefView<'a>> {
        let raw = self.defs.get(id.index())?;
        let path = self.str_at(raw.path_offset());
        let extern_unit = raw.extern_unit_offset().map(|off| self.str_at(off));
        Some(TyDefView { path_hash: raw.path_hash(), path, local_def_idx: raw.local_def_idx(), extern_unit })
    }

    #[inline]
    pub fn get(&self, id: TyId) -> Option<TyKindView<'a>> {
        let e = self.entries.get(id.index())?.from_le();
        Some(self.decode_entry(e))
    }

    #[inline]
    pub fn kind(&self, id: TyId) -> TyKindView<'a> {
        self.get(id).unwrap_or(TyKindView::Unknown)
    }

    #[inline]
    fn words_slice(&self, start: u32, len: u32) -> &'a [u32] {
        let s = start as usize;
        let end = s.saturating_add(len as usize);
        self.args.get(s..end).unwrap_or(&[])
    }

    #[inline]
    fn bounds_slice(&self, start: u32, len: u32) -> &'a [TyBoundEntry] {
        let s = start as usize;
        let end = s.saturating_add(len as usize);
        self.bounds.get(s..end).unwrap_or(&[])
    }

    #[inline]
    fn decode_bound(&self, b: TyBoundEntry) -> TyBoundView<'a> {
        let args_len = u32::from(b.args_len);
        let assoc_words = u32::from(b.assoc_len).saturating_mul(2);
        let args = GenericArgIter { words: self.words_slice(b.args_start, args_len) };
        let assocs = AssocBindingIter { arena: *self, words: self.words_slice(b.args_start.saturating_add(args_len), assoc_words) };
        TyBoundView { trait_def: TyDefId(b.trait_def), is_maybe: (b.flags & BOUND_FLAG_MAYBE) != 0, flags: b.flags, args, assocs }
    }

    fn decode_entry(&self, e: TyEntry) -> TyKindView<'a> {
        match TyTag::from_u8(e.tag) {
            TyTag::Unit => TyKindView::Unit,
            TyTag::Bool => TyKindView::Bool,
            TyTag::Char => TyKindView::Char,
            TyTag::Int => TyKindView::Int(IntTyWidth::from_u8(e.flags)),
            TyTag::Uint => TyKindView::Uint(UintTyWidth::from_u8(e.flags)),
            TyTag::Float => TyKindView::Float(FloatTyWidth::from_u8(e.flags)),
            TyTag::Str => TyKindView::Str,
            TyTag::Never => TyKindView::Never,
            TyTag::Adt => TyKindView::Adt { def: TyDefId(e.a), args: GenericArgIter { words: self.words_slice(e.b, e.c) } },
            TyTag::Ref => TyKindView::Ref { mutability: TyMutability::from_u8(e.flags), inner: TyId(e.a), lifetime: e.b },
            TyTag::Ptr => TyKindView::Ptr { mutability: TyMutability::from_u8(e.flags), inner: TyId(e.a) },
            TyTag::Slice => TyKindView::Slice(TyId(e.a)),
            TyTag::Array => TyKindView::Array { elem: TyId(e.a), len: GenericArgWord::decode_const(e.b) },
            TyTag::Tuple => TyKindView::Tuple(TyListIter { words: self.words_slice(e.a, e.b) }),
            TyTag::FnPtr => TyKindView::FnPtr {
                inputs: TyListIter { words: self.words_slice(e.a, e.b) },
                output: TyId(e.c),
                abi: FnAbiKind::from_u16(e.extra),
                is_unsafe: (e.flags & FN_FLAG_UNSAFE) != 0,
                is_variadic: (e.flags & FN_FLAG_VARIADIC) != 0,
            },
            TyTag::FnDef => TyKindView::FnDef { def: TyDefId(e.a), args: GenericArgIter { words: self.words_slice(e.b, e.c) } },
            TyTag::Param => TyKindView::Param { index: e.a, name: self.str_at(e.b) },
            TyTag::Assoc => {
                let (self_ty, trait_args) = if e.extra == 0 {
                    (TyId(e.c), GenericArgIter::EMPTY)
                } else {
                    let s = self.words_slice(e.c, 1 + u32::from(e.extra));
                    match s.split_first() {
                        Some((&first, rest)) => (TyId(u32::from_le(first)), GenericArgIter { words: rest }),
                        None => (TyId::NONE, GenericArgIter::EMPTY),
                    }
                };
                TyKindView::Assoc { trait_def: TyDefId(e.a), name: self.str_at(e.b), self_ty, trait_args }
            }
            TyTag::Dyn => TyKindView::Dyn { bounds: TyBoundIter { arena: *self, bounds: self.bounds_slice(e.a, e.b) }, lifetime: e.c },
            TyTag::ImplTrait => TyKindView::ImplTrait { bounds: TyBoundIter { arena: *self, bounds: self.bounds_slice(e.a, e.b) } },
            TyTag::Unknown => TyKindView::Unknown,
        }
    }
}
