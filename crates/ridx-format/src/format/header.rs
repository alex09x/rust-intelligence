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

/// Fixed-size header at offset 0 of any .ridx file (format version 3, 160 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// Magic signature ("RIDX")
    pub magic: [u8; 4],
    /// Format version (currently 3)
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
}

/// Fixed-size header for version 2 .ridx files (128 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeaderV2 {
    pub magic: [u8; 4],
    pub version: u32,
    pub crate_name_offset: u32,
    pub crate_version_offset: u32,
    pub string_pool_offset: u64,
    pub string_pool_len: u64,
    pub files_offset: u64,
    pub files_count: u32,
    pub unit_key_offset: u32,
    pub defs_offset: u64,
    pub defs_count: u32,
    pub generation_offset: u32,
    pub defs_by_hash_offset: u64,
    pub defs_by_hash_count: u32,
    pub _pad2: u32,
    pub refs_offset: u64,
    pub refs_count: u32,
    pub _pad3: u32,
    pub refs_by_target_offset: u64,
    pub refs_by_target_count: u32,
    pub _pad4: u32,
    pub types_offset: u64,
    pub types_count: u32,
    pub _pad5: u32,
}

impl HeaderV2 {
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
        }
    }
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
        }
    }

    /// Decodes a version 2 header (128 bytes), setting v3-specific table offsets and counts to 0.
    pub fn from_le_v2(bytes: &[u8]) -> Self {
        let mut b = [0u8; 128];
        b.copy_from_slice(&bytes[..128]);
        let raw: HeaderV2 = unsafe { std::mem::transmute(b) };
        Self {
            magic: raw.magic,
            version: u32::from_le(raw.version),
            crate_name_offset: u32::from_le(raw.crate_name_offset),
            crate_version_offset: u32::from_le(raw.crate_version_offset),
            string_pool_offset: u64::from_le(raw.string_pool_offset),
            string_pool_len: u64::from_le(raw.string_pool_len),
            files_offset: u64::from_le(raw.files_offset),
            files_count: u32::from_le(raw.files_count),
            unit_key_offset: u32::from_le(raw.unit_key_offset),
            defs_offset: u64::from_le(raw.defs_offset),
            defs_count: u32::from_le(raw.defs_count),
            generation_offset: u32::from_le(raw.generation_offset),
            defs_by_hash_offset: u64::from_le(raw.defs_by_hash_offset),
            defs_by_hash_count: u32::from_le(raw.defs_by_hash_count),
            _pad2: 0,
            refs_offset: u64::from_le(raw.refs_offset),
            refs_count: u32::from_le(raw.refs_count),
            _pad3: 0,
            refs_by_target_offset: u64::from_le(raw.refs_by_target_offset),
            refs_by_target_count: u32::from_le(raw.refs_by_target_count),
            _pad4: 0,
            types_offset: u64::from_le(raw.types_offset),
            types_count: u32::from_le(raw.types_count),
            _pad5: 0,
            defs_by_owner_offset: 0,
            defs_by_owner_count: 0,
            _pad6: 0,
            derefs_offset: 0,
            derefs_count: 0,
            _pad7: 0,
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
