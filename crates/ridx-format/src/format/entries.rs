/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::{Span, FLAG_MACRO_EXPANSION};

/// A reference/usage record pointing to a target definition or symbol.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RefEntry {
    /// Source span of this reference usage
    pub span: Span,
    /// Target definition ID if resolved within the same crate, or u32::MAX for external
    pub target_def_id: u32,
    /// Target canonical path offset in string pool (e.g. "std::convert::Into")
    pub target_path_offset: u32,
    /// Target source file path offset in string pool, or 0 if external/unknown
    pub target_file_offset: u32,
    /// Target compilation unit key offset in string pool, or 0 if unknown
    pub target_unit_offset: u32,
    /// Target definition line (1-based), or 0
    pub target_line: u32,
    /// Target definition col (1-based), or 0
    pub target_col: u32,
    /// Kind of symbol being referenced
    pub kind: u8,
    /// Usage flags: 1 = write/mutable, 0 = read/call
    pub is_write: u8,
    /// Flags (bit 0: is_macro_expansion)
    pub flags: u8,
    pub _pad: u8,
}

impl RefEntry {
    #[inline]
    pub fn to_le(self) -> Self {
        Self {
            span: self.span.to_le(),
            target_def_id: self.target_def_id.to_le(),
            target_path_offset: self.target_path_offset.to_le(),
            target_file_offset: self.target_file_offset.to_le(),
            target_unit_offset: self.target_unit_offset.to_le(),
            target_line: self.target_line.to_le(),
            target_col: self.target_col.to_le(),
            kind: self.kind,
            is_write: self.is_write,
            flags: self.flags,
            _pad: self._pad,
        }
    }

    #[inline]
    pub fn from_le(self) -> Self {
        Self {
            span: self.span.from_le(),
            target_def_id: u32::from_le(self.target_def_id),
            target_path_offset: u32::from_le(self.target_path_offset),
            target_file_offset: u32::from_le(self.target_file_offset),
            target_unit_offset: u32::from_le(self.target_unit_offset),
            target_line: u32::from_le(self.target_line),
            target_col: u32::from_le(self.target_col),
            kind: self.kind,
            is_write: self.is_write,
            flags: self.flags,
            _pad: self._pad,
        }
    }

    #[inline]
    pub fn span(&self) -> Span {
        self.span.from_le()
    }
    #[inline]
    pub fn target_def_id(&self) -> u32 {
        u32::from_le(self.target_def_id)
    }
    #[inline]
    pub fn target_path_offset(&self) -> u32 {
        u32::from_le(self.target_path_offset)
    }
    #[inline]
    pub fn target_file_offset(&self) -> u32 {
        u32::from_le(self.target_file_offset)
    }
    #[inline]
    pub fn target_unit_offset(&self) -> u32 {
        u32::from_le(self.target_unit_offset)
    }
    #[inline]
    pub fn target_line(&self) -> u32 {
        u32::from_le(self.target_line)
    }
    #[inline]
    pub fn target_col(&self) -> u32 {
        u32::from_le(self.target_col)
    }
    #[inline]
    pub fn is_macro_expansion(&self) -> bool {
        (self.flags & FLAG_MACRO_EXPANSION) != 0
    }
}

/// Inferred expression type record.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TypeEntry {
    /// Source span of the expression
    pub span: Span,
    /// Offset in string pool of the inferred type name (e.g. "Option<JoinHandle<T>>")
    pub type_offset: u32,
    /// Offset in string pool of the canonical owner path (e.g. "tokio::fs::File"), or 0
    pub owner_path_offset: u32,
}

impl TypeEntry {
    #[inline]
    pub fn new(span: Span, type_offset: u32, owner_path_offset: u32) -> Self {
        Self { span, type_offset, owner_path_offset }
    }

    #[inline]
    pub fn to_le(self) -> Self {
        Self { span: self.span.to_le(), type_offset: self.type_offset.to_le(), owner_path_offset: self.owner_path_offset.to_le() }
    }

    #[inline]
    pub fn from_le(self) -> Self {
        Self { span: self.span.from_le(), type_offset: u32::from_le(self.type_offset), owner_path_offset: u32::from_le(self.owner_path_offset) }
    }

    #[inline]
    pub fn span(&self) -> Span {
        self.span.from_le()
    }
    #[inline]
    pub fn type_offset(&self) -> u32 {
        u32::from_le(self.type_offset)
    }
    #[inline]
    pub fn owner_path_offset(&self) -> u32 {
        u32::from_le(self.owner_path_offset)
    }
}

/// Hash table entry for $O(\log N)$ or binary search lookup by string hash.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HashIndexEntry {
    /// 64-bit FNV-1a hash of the string (path or symbol)
    pub hash: u64,
    /// Index into the corresponding records table (defs or refs)
    pub index: u32,
    pub _pad: [u8; 4],
}

impl HashIndexEntry {
    #[inline]
    pub fn to_le(self) -> Self {
        Self { hash: self.hash.to_le(), index: self.index.to_le(), _pad: self._pad }
    }

    #[inline]
    pub fn from_le(self) -> Self {
        Self { hash: u64::from_le(self.hash), index: u32::from_le(self.index), _pad: self._pad }
    }

    #[inline]
    pub fn hash(&self) -> u64 {
        u64::from_le(self.hash)
    }
    #[inline]
    pub fn index(&self) -> u32 {
        u32::from_le(self.index)
    }
}

/// Definition owner index entry for fast lookup of methods, fields, and associated items by owner.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OwnerIndexEntry {
    /// 64-bit FNV-1a hash of the canonical owner path (e.g. "tokio::fs::File")
    pub hash: u64,
    /// Offset in string pool for the canonical owner path
    pub owner_path_offset: u32,
    /// Index into definitions table
    pub def_index: u32,
}

impl OwnerIndexEntry {
    #[inline]
    pub fn new(hash: u64, owner_path_offset: u32, def_index: u32) -> Self {
        Self { hash, owner_path_offset, def_index }
    }

    #[inline]
    pub fn to_le(self) -> Self {
        Self { hash: self.hash.to_le(), owner_path_offset: self.owner_path_offset.to_le(), def_index: self.def_index.to_le() }
    }

    #[inline]
    pub fn from_le(self) -> Self {
        Self { hash: u64::from_le(self.hash), owner_path_offset: u32::from_le(self.owner_path_offset), def_index: u32::from_le(self.def_index) }
    }

    #[inline]
    pub fn hash(&self) -> u64 {
        u64::from_le(self.hash)
    }
    #[inline]
    pub fn owner_path_offset(&self) -> u32 {
        u32::from_le(self.owner_path_offset)
    }
    #[inline]
    pub fn def_index(&self) -> u32 {
        u32::from_le(self.def_index)
    }
}

/// Deref target mapping entry for resolving autoderef chains (e.g. `String` -> `str`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DerefEntry {
    /// 64-bit FNV-1a hash of the source type canonical path (e.g. "alloc::string::String")
    pub source_type_hash: u64,
    /// Offset in string pool for source type canonical path
    pub source_type_offset: u32,
    /// Offset in string pool for target type canonical path (e.g. "core::primitive::str")
    pub target_type_offset: u32,
}

impl DerefEntry {
    #[inline]
    pub fn new(source_type_hash: u64, source_type_offset: u32, target_type_offset: u32) -> Self {
        Self { source_type_hash, source_type_offset, target_type_offset }
    }

    #[inline]
    pub fn to_le(self) -> Self {
        Self { source_type_hash: self.source_type_hash.to_le(), source_type_offset: self.source_type_offset.to_le(), target_type_offset: self.target_type_offset.to_le() }
    }

    #[inline]
    pub fn from_le(self) -> Self {
        Self {
            source_type_hash: u64::from_le(self.source_type_hash),
            source_type_offset: u32::from_le(self.source_type_offset),
            target_type_offset: u32::from_le(self.target_type_offset),
        }
    }

    #[inline]
    pub fn source_type_hash(&self) -> u64 {
        u64::from_le(self.source_type_hash)
    }
    #[inline]
    pub fn source_type_offset(&self) -> u32 {
        u32::from_le(self.source_type_offset)
    }
    #[inline]
    pub fn target_type_offset(&self) -> u32 {
        u32::from_le(self.target_type_offset)
    }
}
