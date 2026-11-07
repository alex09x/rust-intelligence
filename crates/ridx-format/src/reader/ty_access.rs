/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Zero-copy `IndexReader` accessors for `notes` (v4+) and `TyArena` (v5+) sections.

use crate::format::{FnSigEntry, NoteEntry, TyBoundEntry, TyDefEntry, TyEntry};
use crate::ty_arena::{FnSigView, TyArenaView};

use super::IndexReader;

impl<B: AsRef<[u8]>> IndexReader<B> {
    #[inline]
    fn typed_slice<T>(&self, offset: u64, count: u32) -> &[T] {
        let count = count as usize;
        if count == 0 {
            return &[];
        }
        let start = offset as usize;
        let len = count.saturating_mul(std::mem::size_of::<T>());
        let bytes = self.bytes();
        if start.saturating_add(len) > bytes.len() {
            return &[];
        }
        unsafe { std::slice::from_raw_parts(bytes[start..].as_ptr() as *const T, count) }
    }

    /// Slice of all notes in the index (sorted by span); empty for indexes older than format version 4.
    #[inline]
    pub fn notes(&self) -> &[NoteEntry] {
        let h = self.header();
        self.typed_slice(h.notes_offset, h.notes_count)
    }

    /// The text of the `kind` note whose span starts at `(file_id, line, col)`.
    pub fn note_at(&self, file_id: u32, line: u32, col: u32, kind: u32) -> Option<&str> {
        let notes = self.notes();
        let key = (file_id, line, col);
        let from = notes.partition_point(|n| {
            let s = n.span();
            (s.file_id(), s.start_line(), s.start_col()) < key
        });
        notes[from..]
            .iter()
            .take_while(|n| {
                let s = n.span();
                (s.file_id(), s.start_line(), s.start_col()) == key
            })
            .find(|n| n.kind() == kind)
            .map(|n| self.get_str(n.text_offset()))
    }

    /// Raw slice of `TyDefEntry` records in this shard's `TyArena` (empty for format < v5).
    #[inline]
    pub fn ty_defs(&self) -> &[TyDefEntry] {
        let h = self.header();
        self.typed_slice(h.ty_defs_offset, h.ty_defs_count)
    }

    /// Raw slice of `TyEntry` records in this shard's `TyArena` (empty for format < v5).
    #[inline]
    pub fn ty_entries(&self) -> &[TyEntry] {
        let h = self.header();
        self.typed_slice(h.ty_entries_offset, h.ty_entries_count)
    }

    /// Raw slice of `TyBoundEntry` records in this shard's `TyArena` (empty for format < v5).
    #[inline]
    pub fn ty_bounds(&self) -> &[TyBoundEntry] {
        let h = self.header();
        self.typed_slice(h.ty_bounds_offset, h.ty_bounds_count)
    }

    /// Raw slice of `u32` words in this shard's `TyArena` argument pool (empty for format < v5).
    #[inline]
    pub fn ty_args_raw(&self) -> &[u32] {
        let h = self.header();
        self.typed_slice(h.ty_args_offset, h.ty_args_count)
    }

    /// Zero-copy `TyArenaView` over this shard's interned types and string pool.
    #[inline]
    pub fn ty_arena(&self) -> TyArenaView<'_> {
        let h = self.header();
        let pool_start = h.string_pool_offset as usize;
        let pool_end = pool_start.saturating_add(h.string_pool_len as usize).min(self.bytes().len());
        let pool = self.bytes().get(pool_start..pool_end).unwrap_or(&[]);
        TyArenaView::new(pool, self.ty_defs(), self.ty_entries(), self.ty_bounds(), self.ty_args_raw())
    }

    /// Raw slice of sorted `FnSigEntry` records in this shard (empty for format < v5).
    #[inline]
    pub fn fn_sigs(&self) -> &[FnSigEntry] {
        let h = self.header();
        self.typed_slice(h.fn_sigs_offset, h.fn_sigs_count)
    }

    /// Looks up the structured `FnSigView` for `def_index` in `O(log N)` via binary search.
    #[inline]
    pub fn fn_sig_for_def(&self, def_index: u32) -> Option<FnSigView<'_>> {
        let sigs = self.fn_sigs();
        let pos = sigs.binary_search_by_key(&def_index, FnSigEntry::def_index).ok()?;
        Some(FnSigView::new(sigs[pos], self.ty_args_raw()))
    }
}
