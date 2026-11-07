/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `IndexWriter` builder, sorting, layout, and serialization helpers for `.ridx` v5 typed tables
//! (`TyArena`, `FnSigEntry`, `ImplEntry`, `AssocTyEntry`, `FieldEntry`, `DefTyEntry`,
//! `GenericParamEntry`, `WherePredEntry`).

use std::io::{self, Seek, Write};

use crate::format::{
    AssocTyEntry, CfgEntry, DefTyEntry, FieldEntry, FnSigEntry, GenericOwner, GenericParamEntry, Header, ImplEntry, LangItemEntry, MacroDefEntry, SimplifiedSelfKind, TyBoundEntry,
    TyDefEntry, TyDefId, TyEntry, TyId, UseEntry, WherePredEntry, IMPL_FLAG_TRAIT,
};
use crate::ty_arena::{RawDefTy, RawField, RawFnSig, RawImpl, RawTy, RawTyDef, TyInterner};

use super::encode::{align_up, write_le_items};
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
    pub generic_params: Vec<GenericParamEntry>,
    pub where_preds: Vec<WherePredEntry>,
}

pub(super) struct EncodedTypedSections {
    pub fn_sigs: Vec<FnSigEntry>,
    pub impls: Vec<ImplEntry>,
    pub assoc_tys: Vec<AssocTyEntry>,
    pub fields: Vec<FieldEntry>,
    pub def_tys: Vec<DefTyEntry>,
    pub generic_params: Vec<GenericParamEntry>,
    pub where_preds: Vec<WherePredEntry>,
    pub lang_items: Vec<LangItemEntry>,
    pub uses: Vec<UseEntry>,
    pub cfgs: Vec<CfgEntry>,
    pub macro_defs: Vec<MacroDefEntry>,
    pub ty_defs_offset: u64,
    pub ty_entries_offset: u64,
    pub ty_bounds_offset: u64,
    pub ty_args_offset: u64,
    pub fn_sigs_offset: u64,
    pub impls_offset: u64,
    pub assoc_tys_offset: u64,
    pub fields_offset: u64,
    pub def_tys_offset: u64,
    pub generic_params_offset: u64,
    pub where_preds_offset: u64,
    pub lang_items_offset: u64,
    pub uses_offset: u64,
    pub cfgs_offset: u64,
    pub macro_defs_offset: u64,
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

    /// Records a structured function/method/tuple-ctor signature for `def_index`.
    pub fn add_fn_sig(&mut self, sig: RawFnSig<'_>) {
        let (inputs_start, inputs_len) = self.ty_interner.intern_ty_list(sig.inputs);
        self.fn_sigs.push(FnSigEntry { def_index: sig.def_index, inputs_start, inputs_len, flags: sig.flags, abi: sig.abi as u8, output: sig.output.0 });
    }

    /// Total count of structured function signatures queued.
    pub fn fn_sig_count(&self) -> usize {
        self.fn_sigs.len()
    }

    /// Records a structured inherent or trait `impl` header and its associated type bindings,
    /// returning the queued `pending_impl_idx` handle for attaching generic params and `where` predicates.
    pub fn add_impl(&mut self, imp: RawImpl<'_>) -> u32 {
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

        let pending_idx = self.impls.len() as u32;
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
            generic_params: Vec::new(),
            where_preds: Vec::new(),
        });
        pending_idx
    }

    /// Total count of structured `impl` headers queued.
    pub fn impl_count(&self) -> usize {
        self.impls.len()
    }

    /// Total count of associated type bindings queued across all `impl` headers.
    pub fn assoc_ty_count(&self) -> usize {
        self.impls.iter().map(|i| i.assoc_tys.len()).sum()
    }

    /// Records a structured ADT or enum variant field (`FieldEntry`).
    pub fn add_field(&mut self, field: RawField<'_>) {
        let name_offset = self.intern_string(field.name);
        self.fields.push(FieldEntry {
            owner_def_index: field.owner_def_index,
            def_index: field.def_index.unwrap_or(u32::MAX),
            name_offset,
            ty: field.ty.0,
            field_idx: field.field_idx,
            vis: field.vis as u8,
            flags: field.flags,
        });
    }

    /// Total count of structured fields queued.
    pub fn field_count(&self) -> usize {
        self.fields.len()
    }

    /// Records a structured definition type/constructor entry (`DefTyEntry`).
    pub fn add_def_ty(&mut self, entry: RawDefTy) {
        self.def_tys.push(DefTyEntry {
            def_index: entry.def_index,
            ty: entry.ty.0,
            parent_def_index: entry.parent_def_index.unwrap_or(u32::MAX),
            variant_idx: entry.variant_idx.unwrap_or(u16::MAX),
            ctor_shape: entry.ctor_shape as u8,
            flags: entry.flags,
        });
    }

    /// Total count of structured definition type entries queued.
    pub fn def_ty_count(&self) -> usize {
        self.def_tys.len()
    }

    pub(super) fn prepare_typed_sections(&self, start_offset: u64) -> EncodedTypedSections {
        let mut fn_sigs = self.fn_sigs.clone();
        fn_sigs.sort_by_key(|s| s.def_index);
        fn_sigs.dedup_by_key(|s| s.def_index);

        let mut sorted_impls = self.impls.clone();
        sorted_impls.sort_by_key(|i| {
            let is_trait_u8 = u8::from((i.flags & IMPL_FLAG_TRAIT) != 0);
            (i.simplified_self, is_trait_u8, i.self_ty, i.trait_def, i.def_index)
        });

        let total_assocs: usize = sorted_impls.iter().map(|i| i.assoc_tys.len()).sum();
        let total_impl_gp: usize = sorted_impls.iter().map(|i| i.generic_params.len()).sum();
        let total_impl_wp: usize = sorted_impls.iter().map(|i| i.where_preds.len()).sum();

        let mut generic_params = self.def_generic_params.clone();
        generic_params.sort_by_key(|g| (g.owner_key, g.param_idx));
        generic_params.dedup_by_key(|g| (g.owner_key, g.param_idx));
        generic_params.reserve(total_impl_gp);

        let mut where_preds = self.def_where_preds.clone();
        where_preds.sort_by_key(|w| w.owner_key);
        where_preds.reserve(total_impl_wp);

        let mut impls = Vec::with_capacity(sorted_impls.len());
        let mut assoc_tys = Vec::with_capacity(total_assocs);
        for (impl_idx, mut p) in sorted_impls.into_iter().enumerate() {
            let assoc_tys_start = assoc_tys.len() as u32;
            let assoc_tys_len = u16::try_from(p.assoc_tys.len()).unwrap_or(u16::MAX);
            for a in p.assoc_tys.into_iter().take(usize::from(assoc_tys_len)) {
                assoc_tys.push(AssocTyEntry { impl_index: impl_idx as u32, def_index: a.def_index, name_offset: a.name_offset, ty: a.ty });
            }
            let impl_owner = GenericOwner::Impl(impl_idx as u32).to_raw();
            if !p.generic_params.is_empty() {
                p.generic_params.sort_by_key(|g| g.param_idx);
                p.generic_params.dedup_by_key(|g| g.param_idx);
                for mut gp in p.generic_params {
                    gp.owner_key = impl_owner;
                    generic_params.push(gp);
                }
            }
            for mut wp in p.where_preds {
                wp.owner_key = impl_owner;
                where_preds.push(wp);
            }
            impls.push(ImplEntry {
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

        let mut fields = self.fields.clone();
        fields.sort_by_key(|f| (f.owner_def_index, f.field_idx));
        fields.dedup_by_key(|f| (f.owner_def_index, f.field_idx));

        let mut def_tys = self.def_tys.clone();
        def_tys.sort_by_key(|d| d.def_index);
        def_tys.dedup_by_key(|d| d.def_index);

        let mut lang_items = self.lang_items.clone();
        lang_items.sort_by_key(|l| l.kind);
        lang_items.dedup_by_key(|l| l.kind);

        let uses = self.prepare_sorted_uses();
        let cfgs = self.prepare_sorted_cfgs();
        let macro_defs = self.prepare_sorted_macro_defs();

        let mut cur = start_offset;
        let step = |cur: &mut u64, byte_len: usize| -> u64 {
            let off = *cur;
            *cur = align_up(off + byte_len as u64, 8);
            off
        };

        let ty_defs_offset = step(&mut cur, std::mem::size_of_val(self.ty_interner.defs()));
        let ty_entries_offset = step(&mut cur, std::mem::size_of_val(self.ty_interner.entries()));
        let ty_bounds_offset = step(&mut cur, std::mem::size_of_val(self.ty_interner.bounds()));
        let ty_args_offset = step(&mut cur, std::mem::size_of_val(self.ty_interner.args()));
        let fn_sigs_offset = step(&mut cur, std::mem::size_of_val(fn_sigs.as_slice()));
        let impls_offset = step(&mut cur, std::mem::size_of_val(impls.as_slice()));
        let assoc_tys_offset = step(&mut cur, std::mem::size_of_val(assoc_tys.as_slice()));
        let fields_offset = step(&mut cur, std::mem::size_of_val(fields.as_slice()));
        let def_tys_offset = step(&mut cur, std::mem::size_of_val(def_tys.as_slice()));
        let generic_params_offset = step(&mut cur, std::mem::size_of_val(generic_params.as_slice()));
        let where_preds_offset = step(&mut cur, std::mem::size_of_val(where_preds.as_slice()));
        let lang_items_offset = step(&mut cur, std::mem::size_of_val(lang_items.as_slice()));
        let uses_offset = step(&mut cur, std::mem::size_of_val(uses.as_slice()));
        let cfgs_offset = step(&mut cur, std::mem::size_of_val(cfgs.as_slice()));
        let macro_defs_offset = cur;

        EncodedTypedSections {
            fn_sigs,
            impls,
            assoc_tys,
            fields,
            def_tys,
            generic_params,
            where_preds,
            lang_items,
            uses,
            cfgs,
            macro_defs,
            ty_defs_offset,
            ty_entries_offset,
            ty_bounds_offset,
            ty_args_offset,
            fn_sigs_offset,
            impls_offset,
            assoc_tys_offset,
            fields_offset,
            def_tys_offset,
            generic_params_offset,
            where_preds_offset,
            lang_items_offset,
            uses_offset,
            cfgs_offset,
            macro_defs_offset,
        }
    }

    pub(super) fn fill_typed_header(&self, header: &mut Header, ts: &EncodedTypedSections) {
        header.ty_defs_offset = ts.ty_defs_offset;
        header.ty_defs_count = self.ty_interner.defs().len() as u32;
        header.ty_entries_offset = ts.ty_entries_offset;
        header.ty_entries_count = self.ty_interner.entries().len() as u32;
        header.ty_bounds_offset = ts.ty_bounds_offset;
        header.ty_bounds_count = self.ty_interner.bounds().len() as u32;
        header.ty_args_offset = ts.ty_args_offset;
        header.ty_args_count = self.ty_interner.args().len() as u32;
        header.fn_sigs_offset = ts.fn_sigs_offset;
        header.fn_sigs_count = ts.fn_sigs.len() as u32;
        header.impls_offset = ts.impls_offset;
        header.impls_count = ts.impls.len() as u32;
        header.assoc_tys_offset = ts.assoc_tys_offset;
        header.assoc_tys_count = ts.assoc_tys.len() as u32;
        header.fields_offset = ts.fields_offset;
        header.fields_count = ts.fields.len() as u32;
        header.def_tys_offset = ts.def_tys_offset;
        header.def_tys_count = ts.def_tys.len() as u32;
        header.generic_params_offset = ts.generic_params_offset;
        header.generic_params_count = ts.generic_params.len() as u32;
        header.where_preds_offset = ts.where_preds_offset;
        header.where_preds_count = ts.where_preds.len() as u32;
        header.lang_items_offset = ts.lang_items_offset;
        header.lang_items_count = ts.lang_items.len() as u32;
        header.uses_offset = ts.uses_offset;
        header.uses_count = ts.uses.len() as u32;
        header.cfgs_offset = ts.cfgs_offset;
        header.cfgs_count = ts.cfgs.len() as u32;
        header.macro_defs_offset = ts.macro_defs_offset;
        header.macro_defs_count = ts.macro_defs.len() as u32;
    }

    pub(super) fn write_typed_sections<W: Write + Seek>(&self, writer: &mut W, ts: &EncodedTypedSections) -> io::Result<()> {
        write_le_items(writer, ts.ty_defs_offset, self.ty_interner.defs(), TyDefEntry::to_le)?;
        write_le_items(writer, ts.ty_entries_offset, self.ty_interner.entries(), TyEntry::to_le)?;
        write_le_items(writer, ts.ty_bounds_offset, self.ty_interner.bounds(), TyBoundEntry::to_le)?;
        write_le_items(writer, ts.ty_args_offset, self.ty_interner.args(), u32::to_le)?;
        write_le_items(writer, ts.fn_sigs_offset, &ts.fn_sigs, FnSigEntry::to_le)?;
        write_le_items(writer, ts.impls_offset, &ts.impls, ImplEntry::to_le)?;
        write_le_items(writer, ts.assoc_tys_offset, &ts.assoc_tys, AssocTyEntry::to_le)?;
        write_le_items(writer, ts.fields_offset, &ts.fields, FieldEntry::to_le)?;
        write_le_items(writer, ts.def_tys_offset, &ts.def_tys, DefTyEntry::to_le)?;
        write_le_items(writer, ts.generic_params_offset, &ts.generic_params, GenericParamEntry::to_le)?;
        write_le_items(writer, ts.where_preds_offset, &ts.where_preds, WherePredEntry::to_le)?;
        write_le_items(writer, ts.lang_items_offset, &ts.lang_items, LangItemEntry::to_le)?;
        write_le_items(writer, ts.uses_offset, &ts.uses, UseEntry::to_le)?;
        write_le_items(writer, ts.cfgs_offset, &ts.cfgs, CfgEntry::to_le)?;
        write_le_items(writer, ts.macro_defs_offset, &ts.macro_defs, MacroDefEntry::to_le)?;
        Ok(())
    }
}
