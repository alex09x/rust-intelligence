/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `IndexWriter` builder and sorting helpers for `.ridx` v5 typed tables
//! (`TyArena`, `FnSigEntry`, `ImplEntry`, `AssocTyEntry`).

use crate::format::{AssocTyEntry, FnSigEntry, ImplEntry, SimplifiedSelfKind, TyDefId, TyId, IMPL_FLAG_TRAIT};
use crate::ty_arena::{RawFnSig, RawImpl, RawTy, RawTyDef, TyInterner};

use super::IndexWriter;

#[derive(Debug, Clone)]
pub(super) struct PendingAssocTy {
    pub def_index: u32,
    pub name_offset: u32,
    pub ty: u32,
}

#[derive(Debug, Clone)]
pub(super) struct PendingImpl {
    pub def_index: u32,
    pub self_ty: u32,
    pub trait_def: u32,
    pub trait_args_start: u32,
    pub trait_args_len: u16,
    pub assoc_defs_start: u32,
    pub assoc_defs_len: u16,
    pub flags: u8,
    pub simplified_self: u8,
    pub assoc_tys: Vec<PendingAssocTy>,
}

impl IndexWriter {
    /// Interns a local or external definition handle inside this shard's `TyArena`.
    pub fn intern_ty_def(&mut self, def: RawTyDef<'_>) -> TyDefId {
        let pool = &mut self.string_pool;
        let map = &mut self.string_map;
        self.ty_interner.intern_def(def, |s| Self::intern_in_pool(pool, map, s))
    }

    /// Interns a structured, `Infer`-free type inside this shard's `TyArena`.
    pub fn intern_ty(&mut self, ty: RawTy<'_>) -> TyId {
        let pool = &mut self.string_pool;
        let map = &mut self.string_map;
        self.ty_interner.intern_ty(ty, |s| Self::intern_in_pool(pool, map, s))
    }

    /// Total count of deduplicated `TyEntry` nodes in `TyArena`.
    pub fn ty_entry_count(&self) -> usize {
        self.ty_interner.entries().len()
    }

    /// Read-only access to the `TyInterner` accumulated on this writer.
    pub fn ty_interner(&self) -> &TyInterner {
        &self.ty_interner
    }

    /// Records a structured function/method signature for `def_index`.
    pub fn add_fn_sig(&mut self, sig: RawFnSig<'_>) {
        let (inputs_start, inputs_len) = self.ty_interner.intern_ty_list(sig.inputs);
        self.fn_sigs.push(FnSigEntry { def_index: sig.def_index, inputs_start, inputs_len, flags: sig.flags, abi: sig.abi as u8, output: sig.output.0 });
    }

    /// Total count of structured function signatures queued.
    pub fn fn_sig_count(&self) -> usize {
        self.fn_sigs.len()
    }

    /// Records a structured inherent or trait `impl` header and its associated type bindings.
    pub fn add_impl(&mut self, imp: RawImpl<'_>) {
        let simplified = self.ty_interner.entries().get(imp.self_ty.index()).copied().map_or(SimplifiedSelfKind::Other, SimplifiedSelfKind::from_ty_entry);

        let pool = &mut self.string_pool;
        let map = &mut self.string_map;
        let (trait_args_start, trait_args_len) = self.ty_interner.intern_generic_args_list(imp.trait_args, |s| Self::intern_in_pool(pool, map, s));
        let (assoc_defs_start, assoc_defs_len) = self.ty_interner.intern_u32_list(imp.assoc_defs);

        let mut assoc_tys = Vec::with_capacity(imp.assoc_tys.len());
        for a in imp.assoc_tys {
            let name_offset = self.intern_string(a.name);
            assoc_tys.push(PendingAssocTy { def_index: a.def_index.unwrap_or(u32::MAX), name_offset, ty: a.ty.0 });
        }

        let (trait_def, trait_flag) = match imp.trait_def {
            Some(td) if !td.is_none() => (td.0, IMPL_FLAG_TRAIT),
            _ => (u32::MAX, 0),
        };
        let flags = (imp.flags & !IMPL_FLAG_TRAIT) | trait_flag;

        self.impls.push(PendingImpl {
            def_index: imp.def_index.unwrap_or(u32::MAX),
            self_ty: imp.self_ty.0,
            trait_def,
            trait_args_start,
            trait_args_len,
            assoc_defs_start,
            assoc_defs_len,
            flags,
            simplified_self: simplified as u8,
            assoc_tys,
        });
    }

    /// Total count of structured `impl` headers queued.
    pub fn impl_count(&self) -> usize {
        self.impls.len()
    }

    /// Total count of associated type bindings queued across all `impl` headers.
    pub fn assoc_ty_count(&self) -> usize {
        self.impls.iter().map(|i| i.assoc_tys.len()).sum()
    }

    pub(super) fn build_sorted_typed_tables(&self) -> (Vec<FnSigEntry>, Vec<ImplEntry>, Vec<AssocTyEntry>) {
        let mut fn_sigs = self.fn_sigs.clone();
        fn_sigs.sort_by_key(|s| s.def_index);
        fn_sigs.dedup_by_key(|s| s.def_index);

        let mut sorted_impls = self.impls.clone();
        sorted_impls.sort_by_key(|i| {
            let is_trait_u8 = u8::from((i.flags & IMPL_FLAG_TRAIT) != 0);
            (i.simplified_self, is_trait_u8, i.self_ty, i.trait_def, i.def_index)
        });

        let total_assocs: usize = sorted_impls.iter().map(|i| i.assoc_tys.len()).sum();
        let mut impl_entries = Vec::with_capacity(sorted_impls.len());
        let mut assoc_entries = Vec::with_capacity(total_assocs);

        for (impl_idx, p) in sorted_impls.into_iter().enumerate() {
            let assoc_tys_start = assoc_entries.len() as u32;
            let assoc_tys_len = u16::try_from(p.assoc_tys.len()).unwrap_or(u16::MAX);
            for a in p.assoc_tys.into_iter().take(usize::from(assoc_tys_len)) {
                assoc_entries.push(AssocTyEntry { impl_index: impl_idx as u32, def_index: a.def_index, name_offset: a.name_offset, ty: a.ty });
            }
            impl_entries.push(ImplEntry {
                def_index: p.def_index,
                self_ty: p.self_ty,
                trait_def: p.trait_def,
                trait_args_start: p.trait_args_start,
                assoc_tys_start,
                assoc_defs_start: p.assoc_defs_start,
                trait_args_len: p.trait_args_len,
                assoc_tys_len,
                assoc_defs_len: p.assoc_defs_len,
                flags: p.flags,
                simplified_self: p.simplified_self,
            });
        }

        (fn_sigs, impl_entries, assoc_entries)
    }
}
