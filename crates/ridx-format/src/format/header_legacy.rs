/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Backward-compatible legacy header decoders for `.ridx` versions 2, 3, and 4.

use super::header::Header;

/// Size of a version 3 header (160 bytes): before `notes` and `TyArena` sections.
pub const HEADER_V3_SIZE: usize = 160;

/// Size of a version 4 header (176 bytes): before `TyArena` sections.
pub const HEADER_V4_SIZE: usize = 176;

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
    /// Decodes a version 4 header (176 bytes), zeroing the v5 `TyArena` section descriptors.
    pub fn from_le_v4(bytes: &[u8]) -> Self {
        let mut b = [0u8; std::mem::size_of::<Header>()];
        b[..HEADER_V4_SIZE].copy_from_slice(&bytes[..HEADER_V4_SIZE]);
        let raw: Header = unsafe { std::mem::transmute(b) };
        raw.from_le()
    }

    /// Decodes a version 3 header (160 bytes), zeroing `notes` and `TyArena` section descriptors.
    pub fn from_le_v3(bytes: &[u8]) -> Self {
        let mut b = [0u8; std::mem::size_of::<Header>()];
        b[..HEADER_V3_SIZE].copy_from_slice(&bytes[..HEADER_V3_SIZE]);
        let raw: Header = unsafe { std::mem::transmute(b) };
        raw.from_le()
    }

    /// Decodes a version 2 header (128 bytes), setting v3/v4/v5-specific table offsets and counts to 0.
    pub fn from_le_v2(bytes: &[u8]) -> Self {
        let mut b = [0u8; 128];
        b.copy_from_slice(&bytes[..128]);
        let raw = (unsafe { std::mem::transmute::<[u8; 128], HeaderV2>(b) }).from_le();
        Self {
            magic: raw.magic,
            version: raw.version,
            crate_name_offset: raw.crate_name_offset,
            crate_version_offset: raw.crate_version_offset,
            string_pool_offset: raw.string_pool_offset,
            string_pool_len: raw.string_pool_len,
            files_offset: raw.files_offset,
            files_count: raw.files_count,
            unit_key_offset: raw.unit_key_offset,
            defs_offset: raw.defs_offset,
            defs_count: raw.defs_count,
            generation_offset: raw.generation_offset,
            defs_by_hash_offset: raw.defs_by_hash_offset,
            defs_by_hash_count: raw.defs_by_hash_count,
            refs_offset: raw.refs_offset,
            refs_count: raw.refs_count,
            refs_by_target_offset: raw.refs_by_target_offset,
            refs_by_target_count: raw.refs_by_target_count,
            types_offset: raw.types_offset,
            types_count: raw.types_count,
            ..Self::default()
        }
    }
}
