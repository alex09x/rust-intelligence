/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

/// File table entry: string pool offset for file path, and 64-bit source text provenance hash.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FileEntry {
    pub path_offset: u32,
    pub _pad: u32,
    pub source_hash: u64,
}

impl FileEntry {
    #[inline]
    pub fn new(path_offset: u32, source_hash: u64) -> Self {
        Self { path_offset, _pad: 0, source_hash }
    }

    #[inline]
    pub fn path_offset(&self) -> u32 {
        u32::from_le(self.path_offset)
    }

    #[inline]
    pub fn source_hash(&self) -> u64 {
        u64::from_le(self.source_hash)
    }

    #[inline]
    pub fn to_le(self) -> Self {
        Self { path_offset: self.path_offset.to_le(), _pad: 0, source_hash: self.source_hash.to_le() }
    }
}

/// Fixed-size header at offset 0 of any .ridx file (format version 5, 384 bytes; v4 is 176, v3 is 160).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Header {
    /// Magic signature ("RIDX")
    pub magic: [u8; 4],
    /// Format version (currently 5)
    pub version: u32,
    /// Offset in string pool for crate name
    pub crate_name_offset: u32,
    /// Offset in string pool for crate version
    pub crate_version_offset: u32,

    /// Offset of the string intern pool
    pub string_pool_offset: u64,
    /// Length in bytes of the string intern pool
    pub string_pool_len: u64,

    /// Offset of the file path table (array of FileEntry)
    pub files_offset: u64,
    pub files_count: u32,
    /// Offset in string pool for compilation unit key (or 0)
    pub unit_key_offset: u32,

    /// Offset of the definitions table (array of DefEntry)
    pub defs_offset: u64,
    pub defs_count: u32,
    /// Offset in string pool for source snapshot generation (or 0)
    pub generation_offset: u32,

    /// Offset of the definitions hash index (array of HashIndexEntry sorted by hash)
    pub defs_by_hash_offset: u64,
    pub defs_by_hash_count: u32,
    pub _pad2: u32,

    /// Offset of the references table (array of RefEntry sorted by span)
    pub refs_offset: u64,
    pub refs_count: u32,
    pub _pad3: u32,

    /// Offset of the references-by-target hash index (array of HashIndexEntry sorted by target hash)
    pub refs_by_target_offset: u64,
    pub refs_by_target_count: u32,
    pub _pad4: u32,

    /// Offset of the types table (array of TypeEntry sorted by span)
    pub types_offset: u64,
    pub types_count: u32,
    pub _pad5: u32,

    /// Offset of the definitions-by-owner table (array of OwnerIndexEntry sorted by hash)
    pub defs_by_owner_offset: u64,
    pub defs_by_owner_count: u32,
    pub _pad6: u32,

    /// Offset of the deref mapping table (array of DerefEntry sorted by source_type_hash)
    pub derefs_offset: u64,
    pub derefs_count: u32,
    pub _pad7: u32,

    /// Offset of the notes table (array of NoteEntry sorted by span); version 4.
    pub notes_offset: u64,
    pub notes_count: u32,
    pub _pad8: u32,

    /// Offset of the `TyArena` definition table (`TyDefEntry`); version 5.
    pub ty_defs_offset: u64,
    pub ty_defs_count: u32,
    pub _pad9: u32,

    /// Offset of the `TyArena` interned type table (`TyEntry`); version 5.
    pub ty_entries_offset: u64,
    pub ty_entries_count: u32,
    pub _pad10: u32,

    /// Offset of the `TyArena` trait bound table (`TyBoundEntry`); version 5.
    pub ty_bounds_offset: u64,
    pub ty_bounds_count: u32,
    pub _pad11: u32,

    /// Offset of the `TyArena` packed generic argument pool (`u32`); version 5.
    pub ty_args_offset: u64,
    pub ty_args_count: u32,
    pub _pad12: u32,

    /// Offset of the structured `FnSigEntry` table (sorted by `def_index`); version 5.
    pub fn_sigs_offset: u64,
    pub fn_sigs_count: u32,
    pub _pad13: u32,

    /// Offset of the structured `ImplEntry` table (sorted by `simplified_self`); version 5.
    pub impls_offset: u64,
    pub impls_count: u32,
    pub _pad14: u32,

    /// Offset of the `AssocTyEntry` table (contiguous per `ImplEntry`); version 5.
    pub assoc_tys_offset: u64,
    pub assoc_tys_count: u32,
    pub _pad15: u32,

    /// Offset of the `FieldEntry` table (sorted by `(owner_def_index, field_idx)`); version 5.
    pub fields_offset: u64,
    pub fields_count: u32,
    pub _pad16: u32,

    /// Offset of the `DefTyEntry` table (sorted by `def_index`); version 5.
    pub def_tys_offset: u64,
    pub def_tys_count: u32,
    pub _pad17: u32,

    /// Offset of the `GenericParamEntry` table (sorted by `(owner_key, param_idx)`); version 5.
    pub generic_params_offset: u64,
    pub generic_params_count: u32,
    pub _pad18: u32,

    /// Offset of the `WherePredEntry` table (sorted by `owner_key`); version 5.
    pub where_preds_offset: u64,
    pub where_preds_count: u32,
    pub _pad19: u32,

    /// Offset of the `LangItemEntry` table (sorted by `kind`); version 5.
    pub lang_items_offset: u64,
    pub lang_items_count: u32,
    pub _pad20: u32,

    /// Offset of the `UseEntry` table (sorted by `(owner_raw, name_str)`); version 5.
    pub uses_offset: u64,
    pub uses_count: u32,
    pub _pad21: u32,
}

impl Header {
    #[inline]
    pub fn to_le(self) -> Self {
        Self {
            magic: self.magic,
            version: self.version.to_le(),
            crate_name_offset: self.crate_name_offset.to_le(),
            crate_version_offset: self.crate_version_offset.to_le(),
            string_pool_offset: self.string_pool_offset.to_le(),
            string_pool_len: self.string_pool_len.to_le(),
            files_offset: self.files_offset.to_le(),
            files_count: self.files_count.to_le(),
            unit_key_offset: self.unit_key_offset.to_le(),
            defs_offset: self.defs_offset.to_le(),
            defs_count: self.defs_count.to_le(),
            generation_offset: self.generation_offset.to_le(),
            defs_by_hash_offset: self.defs_by_hash_offset.to_le(),
            defs_by_hash_count: self.defs_by_hash_count.to_le(),
            _pad2: 0,
            refs_offset: self.refs_offset.to_le(),
            refs_count: self.refs_count.to_le(),
            _pad3: 0,
            refs_by_target_offset: self.refs_by_target_offset.to_le(),
            refs_by_target_count: self.refs_by_target_count.to_le(),
            _pad4: 0,
            types_offset: self.types_offset.to_le(),
            types_count: self.types_count.to_le(),
            _pad5: 0,
            defs_by_owner_offset: self.defs_by_owner_offset.to_le(),
            defs_by_owner_count: self.defs_by_owner_count.to_le(),
            _pad6: 0,
            derefs_offset: self.derefs_offset.to_le(),
            derefs_count: self.derefs_count.to_le(),
            _pad7: 0,
            notes_offset: self.notes_offset.to_le(),
            notes_count: self.notes_count.to_le(),
            _pad8: 0,
            ty_defs_offset: self.ty_defs_offset.to_le(),
            ty_defs_count: self.ty_defs_count.to_le(),
            _pad9: 0,
            ty_entries_offset: self.ty_entries_offset.to_le(),
            ty_entries_count: self.ty_entries_count.to_le(),
            _pad10: 0,
            ty_bounds_offset: self.ty_bounds_offset.to_le(),
            ty_bounds_count: self.ty_bounds_count.to_le(),
            _pad11: 0,
            ty_args_offset: self.ty_args_offset.to_le(),
            ty_args_count: self.ty_args_count.to_le(),
            _pad12: 0,
            fn_sigs_offset: self.fn_sigs_offset.to_le(),
            fn_sigs_count: self.fn_sigs_count.to_le(),
            _pad13: 0,
            impls_offset: self.impls_offset.to_le(),
            impls_count: self.impls_count.to_le(),
            _pad14: 0,
            assoc_tys_offset: self.assoc_tys_offset.to_le(),
            assoc_tys_count: self.assoc_tys_count.to_le(),
            _pad15: 0,
            fields_offset: self.fields_offset.to_le(),
            fields_count: self.fields_count.to_le(),
            _pad16: 0,
            def_tys_offset: self.def_tys_offset.to_le(),
            def_tys_count: self.def_tys_count.to_le(),
            _pad17: 0,
            generic_params_offset: self.generic_params_offset.to_le(),
            generic_params_count: self.generic_params_count.to_le(),
            _pad18: 0,
            where_preds_offset: self.where_preds_offset.to_le(),
            where_preds_count: self.where_preds_count.to_le(),
            _pad19: 0,
            lang_items_offset: self.lang_items_offset.to_le(),
            lang_items_count: self.lang_items_count.to_le(),
            _pad20: 0,
            uses_offset: self.uses_offset.to_le(),
            uses_count: self.uses_count.to_le(),
            _pad21: 0,
        }
    }

    #[inline]
    pub fn from_le(self) -> Self {
        Self {
            magic: self.magic,
            version: u32::from_le(self.version),
            crate_name_offset: u32::from_le(self.crate_name_offset),
            crate_version_offset: u32::from_le(self.crate_version_offset),
            string_pool_offset: u64::from_le(self.string_pool_offset),
            string_pool_len: u64::from_le(self.string_pool_len),
            files_offset: u64::from_le(self.files_offset),
            files_count: u32::from_le(self.files_count),
            unit_key_offset: u32::from_le(self.unit_key_offset),
            defs_offset: u64::from_le(self.defs_offset),
            defs_count: u32::from_le(self.defs_count),
            generation_offset: u32::from_le(self.generation_offset),
            defs_by_hash_offset: u64::from_le(self.defs_by_hash_offset),
            defs_by_hash_count: u32::from_le(self.defs_by_hash_count),
            _pad2: 0,
            refs_offset: u64::from_le(self.refs_offset),
            refs_count: u32::from_le(self.refs_count),
            _pad3: 0,
            refs_by_target_offset: u64::from_le(self.refs_by_target_offset),
            refs_by_target_count: u32::from_le(self.refs_by_target_count),
            _pad4: 0,
            types_offset: u64::from_le(self.types_offset),
            types_count: u32::from_le(self.types_count),
            _pad5: 0,
            defs_by_owner_offset: u64::from_le(self.defs_by_owner_offset),
            defs_by_owner_count: u32::from_le(self.defs_by_owner_count),
            _pad6: 0,
            derefs_offset: u64::from_le(self.derefs_offset),
            derefs_count: u32::from_le(self.derefs_count),
            _pad7: 0,
            notes_offset: u64::from_le(self.notes_offset),
            notes_count: u32::from_le(self.notes_count),
            _pad8: 0,
            ty_defs_offset: u64::from_le(self.ty_defs_offset),
            ty_defs_count: u32::from_le(self.ty_defs_count),
            _pad9: 0,
            ty_entries_offset: u64::from_le(self.ty_entries_offset),
            ty_entries_count: u32::from_le(self.ty_entries_count),
            _pad10: 0,
            ty_bounds_offset: u64::from_le(self.ty_bounds_offset),
            ty_bounds_count: u32::from_le(self.ty_bounds_count),
            _pad11: 0,
            ty_args_offset: u64::from_le(self.ty_args_offset),
            ty_args_count: u32::from_le(self.ty_args_count),
            _pad12: 0,
            fn_sigs_offset: u64::from_le(self.fn_sigs_offset),
            fn_sigs_count: u32::from_le(self.fn_sigs_count),
            _pad13: 0,
            impls_offset: u64::from_le(self.impls_offset),
            impls_count: u32::from_le(self.impls_count),
            _pad14: 0,
            assoc_tys_offset: u64::from_le(self.assoc_tys_offset),
            assoc_tys_count: u32::from_le(self.assoc_tys_count),
            _pad15: 0,
            fields_offset: u64::from_le(self.fields_offset),
            fields_count: u32::from_le(self.fields_count),
            _pad16: 0,
            def_tys_offset: u64::from_le(self.def_tys_offset),
            def_tys_count: u32::from_le(self.def_tys_count),
            _pad17: 0,
            generic_params_offset: u64::from_le(self.generic_params_offset),
            generic_params_count: u32::from_le(self.generic_params_count),
            _pad18: 0,
            where_preds_offset: u64::from_le(self.where_preds_offset),
            where_preds_count: u32::from_le(self.where_preds_count),
            _pad19: 0,
            lang_items_offset: u64::from_le(self.lang_items_offset),
            lang_items_count: u32::from_le(self.lang_items_count),
            _pad20: 0,
            uses_offset: u64::from_le(self.uses_offset),
            uses_count: u32::from_le(self.uses_count),
            _pad21: 0,
        }
    }

    #[inline]
    pub fn unit_key_offset(&self) -> u32 {
        u32::from_le(self.unit_key_offset)
    }

    #[inline]
    pub fn generation_offset(&self) -> u32 {
        u32::from_le(self.generation_offset)
    }
}
