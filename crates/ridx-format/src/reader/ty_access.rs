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

use crate::format::{
    AssocTyEntry, DefTyEntry, FieldEntry, FnSigEntry, GenericOwner, GenericParamEntry, ImplEntry, NoteEntry, SimplifiedSelfKind, TyBoundEntry, TyDefEntry, TyEntry, WherePredEntry,
};
use crate::ty_arena::{DefTyView, FieldIter, FieldView, FnSigView, GenericParamIter, ImplIter, ImplView, TyArenaView, WherePredIter};

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

    /// Raw slice of sorted `ImplEntry` records in this shard (empty for format < v5).
    #[inline]
    pub fn impls(&self) -> &[ImplEntry] {
        let h = self.header();
        self.typed_slice(h.impls_offset, h.impls_count)
    }

    /// Raw slice of contiguous `AssocTyEntry` bindings in this shard (empty for format < v5).
    #[inline]
    pub fn assoc_tys(&self) -> &[AssocTyEntry] {
        let h = self.header();
        self.typed_slice(h.assoc_tys_offset, h.assoc_tys_count)
    }

    /// Decodes the `ImplView` at `impl_index` (`0..impls().len()`).
    #[inline]
    pub fn impl_view(&self, impl_index: u32) -> Option<ImplView<'_>> {
        let raw = *self.impls().get(impl_index as usize)?;
        Some(ImplView::new(impl_index, raw, self.ty_arena(), self.assoc_tys()))
    }

    /// Iterates over all `ImplView` headers in this shard.
    #[inline]
    pub fn impls_iter(&self) -> ImplIter<'_> {
        ImplIter::new(0, self.impls(), self.ty_arena(), self.assoc_tys())
    }

    /// Binary-searches all `ImplEntry` records (both inherent and trait) whose `self_ty` matches `kind`.
    #[inline]
    pub fn impls_for_simplified(&self, kind: SimplifiedSelfKind) -> ImplIter<'_> {
        let all = self.impls();
        let k = kind as u8;
        let start = all.partition_point(|e| e.simplified_self < k);
        let end = start + all[start..].partition_point(|e| e.simplified_self == k);
        ImplIter::new(start as u32, &all[start..end], self.ty_arena(), self.assoc_tys())
    }

    /// Binary-searches inherent `impl Self` blocks whose `self_ty` matches `kind`.
    #[inline]
    pub fn inherent_impls_for_simplified(&self, kind: SimplifiedSelfKind) -> ImplIter<'_> {
        self.impls_for_bucket(kind, false)
    }

    /// Binary-searches `impl Trait for Self` blocks whose `self_ty` matches `kind`.
    #[inline]
    pub fn trait_impls_for_simplified(&self, kind: SimplifiedSelfKind) -> ImplIter<'_> {
        self.impls_for_bucket(kind, true)
    }

    #[inline]
    fn impls_for_bucket(&self, kind: SimplifiedSelfKind, want_trait: bool) -> ImplIter<'_> {
        let all = self.impls();
        let key = (kind as u8, u8::from(want_trait));
        let start = all.partition_point(|e| (e.simplified_self, u8::from(e.is_trait())) < key);
        let end = start + all[start..].partition_point(|e| (e.simplified_self, u8::from(e.is_trait())) == key);
        ImplIter::new(start as u32, &all[start..end], self.ty_arena(), self.assoc_tys())
    }

    /// Raw slice of sorted `FieldEntry` records in this shard (empty for format < v5).
    #[inline]
    pub fn fields(&self) -> &[FieldEntry] {
        let h = self.header();
        self.typed_slice(h.fields_offset, h.fields_count)
    }

    /// Binary-searches all `FieldEntry` rows belonging to `owner_def_index` (`Struct`, `Union`, or `Variant`).
    #[inline]
    pub fn fields_for_owner(&self, owner_def_index: u32) -> FieldIter<'_> {
        let all = self.fields();
        let start = all.partition_point(|e| e.owner_def_index() < owner_def_index);
        let end = start + all[start..].partition_point(|e| e.owner_def_index() == owner_def_index);
        FieldIter::new(self.ty_arena(), &all[start..end])
    }

    /// Looks up a specific field `name` (or positional `"0"`, `"1"`) on `owner_def_index`.
    #[inline]
    pub fn find_field(&self, owner_def_index: u32, name: &str) -> Option<FieldView<'_>> {
        self.fields_for_owner(owner_def_index).find(|f| f.name == name)
    }

    /// Raw slice of sorted `DefTyEntry` records in this shard (empty for format < v5).
    #[inline]
    pub fn def_tys(&self) -> &[DefTyEntry] {
        let h = self.header();
        self.typed_slice(h.def_tys_offset, h.def_tys_count)
    }

    /// Looks up the `DefTyView` (`TyAlias` RHS, `Const`/`Static`/`AssocConst` type, or ADT/variant ctor shape)
    /// for `def_index` in `O(log N)` via binary search.
    #[inline]
    pub fn def_ty_for_def(&self, def_index: u32) -> Option<DefTyView> {
        let all = self.def_tys();
        let pos = all.binary_search_by_key(&def_index, DefTyEntry::def_index).ok()?;
        Some(DefTyView::new(all[pos]))
    }

    /// Raw slice of sorted `GenericParamEntry` records in this shard (empty for format < v5).
    #[inline]
    pub fn generic_params(&self) -> &[GenericParamEntry] {
        let h = self.header();
        self.typed_slice(h.generic_params_offset, h.generic_params_count)
    }

    /// Binary-searches all `GenericParamEntry` declarations owned by `owner` (`Def` or `Impl`).
    #[inline]
    pub fn generic_params_for_owner(&self, owner: GenericOwner) -> GenericParamIter<'_> {
        let all = self.generic_params();
        let key = owner.to_raw();
        let start = all.partition_point(|e| e.owner_key() < key);
        let end = start + all[start..].partition_point(|e| e.owner_key() == key);
        GenericParamIter::new(self.ty_arena(), &all[start..end])
    }

    /// Binary-searches all `GenericParamEntry` declarations owned by `DefEntry` `def_index`.
    #[inline]
    pub fn generic_params_for_def(&self, def_index: u32) -> GenericParamIter<'_> {
        self.generic_params_for_owner(GenericOwner::Def(def_index))
    }

    /// Binary-searches all `GenericParamEntry` declarations owned by `ImplEntry` `impl_index`.
    #[inline]
    pub fn generic_params_for_impl(&self, impl_index: u32) -> GenericParamIter<'_> {
        self.generic_params_for_owner(GenericOwner::Impl(impl_index))
    }

    /// Raw slice of sorted `WherePredEntry` records in this shard (empty for format < v5).
    #[inline]
    pub fn where_preds(&self) -> &[WherePredEntry] {
        let h = self.header();
        self.typed_slice(h.where_preds_offset, h.where_preds_count)
    }

    /// Binary-searches all `WherePredEntry` bounds/predicates owned by `owner` (`Def` or `Impl`).
    #[inline]
    pub fn where_preds_for_owner(&self, owner: GenericOwner) -> WherePredIter<'_> {
        let all = self.where_preds();
        let key = owner.to_raw();
        let start = all.partition_point(|e| e.owner_key() < key);
        let end = start + all[start..].partition_point(|e| e.owner_key() == key);
        WherePredIter::new(self.ty_arena(), &all[start..end])
    }

    /// Binary-searches all `WherePredEntry` bounds/predicates owned by `DefEntry` `def_index`.
    #[inline]
    pub fn where_preds_for_def(&self, def_index: u32) -> WherePredIter<'_> {
        self.where_preds_for_owner(GenericOwner::Def(def_index))
    }

    /// Binary-searches all `WherePredEntry` bounds/predicates owned by `ImplEntry` `impl_index`.
    #[inline]
    pub fn where_preds_for_impl(&self, impl_index: u32) -> WherePredIter<'_> {
        self.where_preds_for_owner(GenericOwner::Impl(impl_index))
    }
}
