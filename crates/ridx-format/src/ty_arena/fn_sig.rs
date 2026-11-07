/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Zero-copy reader view and builder input for structured function signatures (`FnSigEntry`).

use crate::format::{FnAbiKind, FnSigEntry, TyId, FN_FLAG_ASYNC, FN_FLAG_CONST, FN_FLAG_HAS_SELF, FN_FLAG_UNSAFE, FN_FLAG_VARIADIC};

use super::TyListIter;

/// Builder input for `IndexWriter::add_fn_sig`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawFnSig<'a> {
    pub def_index: u32,
    pub inputs: &'a [TyId],
    pub output: TyId,
    pub abi: FnAbiKind,
    pub flags: u8,
}

/// Zero-copy view over a `FnSigEntry` and its parameter `TyId` slice in `ty_args`.
#[derive(Debug, Clone, Copy)]
pub struct FnSigView<'a> {
    entry: FnSigEntry,
    ty_args: &'a [u32],
}

impl<'a> FnSigView<'a> {
    #[inline]
    pub const fn new(entry: FnSigEntry, ty_args: &'a [u32]) -> Self {
        Self { entry: entry.from_le(), ty_args }
    }

    #[inline]
    pub const fn def_index(&self) -> u32 {
        self.entry.def_index
    }

    #[inline]
    pub const fn output(&self) -> TyId {
        TyId(self.entry.output)
    }

    #[inline]
    pub const fn abi(&self) -> FnAbiKind {
        FnAbiKind::from_u8(self.entry.abi)
    }

    #[inline]
    pub const fn flags(&self) -> u8 {
        self.entry.flags
    }

    #[inline]
    pub const fn is_unsafe(&self) -> bool {
        (self.entry.flags & FN_FLAG_UNSAFE) != 0
    }

    #[inline]
    pub const fn is_variadic(&self) -> bool {
        (self.entry.flags & FN_FLAG_VARIADIC) != 0
    }

    #[inline]
    pub const fn is_async(&self) -> bool {
        (self.entry.flags & FN_FLAG_ASYNC) != 0
    }

    #[inline]
    pub const fn is_const(&self) -> bool {
        (self.entry.flags & FN_FLAG_CONST) != 0
    }

    #[inline]
    pub const fn has_self(&self) -> bool {
        (self.entry.flags & FN_FLAG_HAS_SELF) != 0
    }

    #[inline]
    pub fn inputs(&self) -> TyListIter<'a> {
        let start = self.entry.inputs_start as usize;
        let end = start.saturating_add(self.entry.inputs_len as usize);
        let words = self.ty_args.get(start..end).unwrap_or(&[]);
        TyListIter::from_words(words)
    }

    #[inline]
    pub const fn inputs_len(&self) -> usize {
        self.entry.inputs_len as usize
    }
}
