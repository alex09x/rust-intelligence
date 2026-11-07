/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Fast zero-copy memory-mapped reader for .ridx files.
#![allow(clippy::manual_is_multiple_of)]

use memmap2::Mmap;
use std::fs::File;
use std::io;
use std::path::Path;

use crate::format::*;

mod cfg_access;
mod definition_at;
mod file_lookup;
mod macro_access;
mod name_reference;
mod queries;
mod ty_access;
mod use_access;
pub(crate) use self::definition_at::is_synthetic_def_name;
pub use self::macro_access::MacroDefView;
pub(crate) use self::name_reference::span_strictly_inside;
mod search;

use self::search::validate_sections;
#[cfg(test)]
use self::search::{check_section, contains_ignore_case_ascii};
/// Representation of a found definition target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDefinition<'a> {
    pub name: &'a str,
    pub path: &'a str,
    pub kind: DefKind,
    pub signature: Option<&'a str>,
    pub file: &'a str,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub is_macro_expansion: bool,
    pub is_reexport: bool,
    pub source_root: Option<&'a Path>,
    pub unit_key: Option<&'a str>,
    pub reader_idx: Option<usize>,
}

impl<'a> ResolvedDefinition<'a> {
    pub fn resolved_path(&self) -> std::path::PathBuf {
        let p = Path::new(self.file);
        if p.is_absolute() {
            p.to_path_buf()
        } else if let Some(root) = self.source_root {
            join_resolving_overlap(root, p)
        } else {
            p.to_path_buf()
        }
    }
}

/// Zero-copy reader over a memory-mapped or buffered `.ridx` index.
#[derive(Debug)]
pub struct IndexReader<B> {
    buffer: B,
    header: Header,
}

impl IndexReader<Mmap> {
    /// Opens and memory-maps a `.ridx` file from disk.
    pub fn open(path: impl AsRef<Path>) -> io::Result<IndexReader<Mmap>> {
        let file = File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };
        Self::from_buffer(mmap)
    }
}

impl<B: AsRef<[u8]>> IndexReader<B> {
    /// Wraps any byte buffer as an IndexReader.
    pub fn from_buffer(buffer: B) -> io::Result<Self> {
        let bytes = buffer.as_ref();
        if (bytes.as_ptr() as usize) % 8 != 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Buffer base pointer must be 8-byte aligned for zero-copy RIDX reading"));
        }
        if bytes.len() < std::mem::size_of::<HeaderV2>() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "File too small for RIDX header"));
        }

        let magic: [u8; 4] = bytes[0..4].try_into().unwrap();
        if magic != MAGIC {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Invalid RIDX magic: {:?}", magic)));
        }

        let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let header = match version {
            2 => Header::from_le_v2(&bytes[..std::mem::size_of::<HeaderV2>()]),
            3 => {
                if bytes.len() < crate::format::HEADER_V3_SIZE {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "File too small for RIDX header"));
                }
                Header::from_le_v3(bytes)
            }
            4 => {
                if bytes.len() < crate::format::HEADER_V4_SIZE {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "File too small for RIDX header"));
                }
                Header::from_le_v4(bytes)
            }
            CURRENT_VERSION => {
                if bytes.len() < std::mem::size_of::<Header>() {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "File too small for RIDX header"));
                }
                let mut header_buf = [0u8; std::mem::size_of::<Header>()];
                header_buf.copy_from_slice(&bytes[..std::mem::size_of::<Header>()]);
                let raw_header: Header = unsafe { std::mem::transmute(header_buf) };
                raw_header.from_le()
            }
            _ => {
                return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Unsupported RIDX version: {}", version)));
            }
        };

        validate_sections(bytes.len(), &header)?;

        Ok(Self { buffer, header })
    }

    #[inline]
    pub(super) fn bytes(&self) -> &[u8] {
        self.buffer.as_ref()
    }

    #[inline]
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// Crate name stored in the index.
    pub fn crate_name(&self) -> &str {
        self.get_str(self.header.crate_name_offset)
    }

    /// Crate version stored in the index.
    pub fn crate_version(&self) -> &str {
        self.get_str(self.header.crate_version_offset)
    }

    /// Compilation unit key stored in the index (or empty string if none).
    pub fn unit_key(&self) -> &str {
        let offset = self.header.unit_key_offset();
        if offset == 0 {
            ""
        } else {
            self.get_str(offset)
        }
    }

    /// Source snapshot generation digest stored in the index (or empty string if none).
    pub fn generation(&self) -> &str {
        let offset = self.header.generation_offset();
        if offset == 0 {
            ""
        } else {
            self.get_str(offset)
        }
    }

    /// Target compilation unit key for a reference (or empty string if none).
    pub fn target_unit(&self, r: &RefEntry) -> &str {
        self.get_str(r.target_unit_offset())
    }

    /// Retrieves an interned null-terminated string at the specified offset.
    #[inline]
    pub fn get_str(&self, offset: u32) -> &str {
        let pool_start = self.header.string_pool_offset as usize;
        let pool_len = self.header.string_pool_len as usize;
        let off = offset as usize;
        if off >= pool_len {
            return "";
        }
        let bytes = self.bytes();
        let start = pool_start + off;
        let max_end = (pool_start + pool_len).min(bytes.len());
        if start >= max_end {
            return "";
        }
        let end = bytes[start..max_end].iter().position(|&b| b == 0).map(|p| start + p).unwrap_or(max_end);

        std::str::from_utf8(&bytes[start..end]).unwrap_or("")
    }

    /// Returns the file path for a given file_id.
    pub fn get_file(&self, file_id: u32) -> &str {
        if file_id >= self.header.files_count {
            return "";
        }
        let offset_in_file_table = (self.header.files_offset as usize) + (file_id as usize) * std::mem::size_of::<crate::format::FileEntry>();
        let bytes = self.bytes();
        if offset_in_file_table + std::mem::size_of::<crate::format::FileEntry>() > bytes.len() {
            return "";
        }
        let str_offset = u32::from_le_bytes(bytes[offset_in_file_table..offset_in_file_table + 4].try_into().unwrap());
        self.get_str(str_offset)
    }

    /// Returns the compiled source text hash for a given file_id embedded directly in this index.
    pub fn file_source_hash(&self, file_id: u32) -> u64 {
        if file_id >= self.header.files_count {
            return 0;
        }
        let offset_in_file_table = (self.header.files_offset as usize) + (file_id as usize) * std::mem::size_of::<crate::format::FileEntry>();
        let bytes = self.bytes();
        if offset_in_file_table + std::mem::size_of::<crate::format::FileEntry>() > bytes.len() {
            return 0;
        }
        u64::from_le_bytes(bytes[offset_in_file_table + 8..offset_in_file_table + 16].try_into().unwrap())
    }

    /// Returns all source file provenance hashes embedded directly inside this index shard.
    pub fn embedded_file_hashes(&self) -> std::collections::HashMap<String, u64> {
        let mut map = std::collections::HashMap::with_capacity(self.header.files_count as usize);
        for id in 0..self.header.files_count {
            let f = self.get_file(id);
            let h = self.file_source_hash(id);
            if !f.is_empty() && h != 0 {
                map.insert(f.to_string(), h);
            }
        }
        map
    }

    /// Total count of source files in the index.
    #[inline]
    pub fn files_count(&self) -> u32 {
        self.header.files_count
    }

    /// Slice of all definitions in the index.
    #[inline]
    pub fn definitions(&self) -> &[DefEntry] {
        let header = self.header();
        let start = header.defs_offset as usize;
        let count = header.defs_count as usize;
        let len = count * std::mem::size_of::<DefEntry>();
        let bytes = self.bytes();
        if start + len > bytes.len() {
            return &[];
        }
        unsafe { std::slice::from_raw_parts(bytes[start..].as_ptr() as *const DefEntry, count) }
    }

    #[inline]
    pub(crate) fn defs_by_hash(&self) -> &[HashIndexEntry] {
        let header = self.header();
        let start = header.defs_by_hash_offset as usize;
        let count = header.defs_by_hash_count as usize;
        let len = count * std::mem::size_of::<HashIndexEntry>();
        let bytes = self.bytes();
        if start + len > bytes.len() {
            return &[];
        }
        unsafe { std::slice::from_raw_parts(bytes[start..].as_ptr() as *const HashIndexEntry, count) }
    }

    /// Slice of all references in the index (sorted by source span).
    #[inline]
    pub fn references(&self) -> &[RefEntry] {
        let header = self.header();
        let start = header.refs_offset as usize;
        let count = header.refs_count as usize;
        let len = count * std::mem::size_of::<RefEntry>();
        let bytes = self.bytes();
        if start + len > bytes.len() {
            return &[];
        }
        unsafe { std::slice::from_raw_parts(bytes[start..].as_ptr() as *const RefEntry, count) }
    }

    #[inline]
    fn refs_by_target(&self) -> &[HashIndexEntry] {
        let header = self.header();
        let start = header.refs_by_target_offset as usize;
        let count = header.refs_by_target_count as usize;
        let len = count * std::mem::size_of::<HashIndexEntry>();
        let bytes = self.bytes();
        if start + len > bytes.len() {
            return &[];
        }
        unsafe { std::slice::from_raw_parts(bytes[start..].as_ptr() as *const HashIndexEntry, count) }
    }

    /// Slice of all expression types in the index (sorted by source span).
    #[inline]
    pub fn types(&self) -> &[TypeEntry] {
        let header = self.header();
        let start = header.types_offset as usize;
        let count = header.types_count as usize;
        let len = count * std::mem::size_of::<TypeEntry>();
        let bytes = self.bytes();
        if start + len > bytes.len() {
            return &[];
        }
        unsafe { std::slice::from_raw_parts(bytes[start..].as_ptr() as *const TypeEntry, count) }
    }

    /// Slice of all definitions-by-owner entries in the index (sorted by owner hash).
    #[inline]
    pub fn defs_by_owner(&self) -> &[OwnerIndexEntry] {
        let header = self.header();
        let count = header.defs_by_owner_count as usize;
        if count == 0 {
            return &[];
        }
        let start = header.defs_by_owner_offset as usize;
        let len = count * std::mem::size_of::<OwnerIndexEntry>();
        let bytes = self.bytes();
        if start + len > bytes.len() {
            return &[];
        }
        unsafe { std::slice::from_raw_parts(bytes[start..].as_ptr() as *const OwnerIndexEntry, count) }
    }

    /// Slice of all deref entries in the index (sorted by source_type_hash).
    #[inline]
    pub fn deref_entries(&self) -> &[DerefEntry] {
        let header = self.header();
        let count = header.derefs_count as usize;
        if count == 0 {
            return &[];
        }
        let start = header.derefs_offset as usize;
        let len = count * std::mem::size_of::<DerefEntry>();
        let bytes = self.bytes();
        if start + len > bytes.len() {
            return &[];
        }
        unsafe { std::slice::from_raw_parts(bytes[start..].as_ptr() as *const DerefEntry, count) }
    }
}

#[cfg(test)]
pub(crate) mod cursor_precedence_tests;
#[cfg(test)]
mod file_id_exact_tests;
#[cfg(test)]
mod name_span_tests;
#[cfg(test)]
mod notes_tests;
#[cfg(test)]
mod owner_and_deref_tests;
#[cfg(test)]
pub(crate) mod root_module_tests;
#[cfg(test)]
mod search_symbols_tests;
#[cfg(test)]
mod spans_and_sections_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod type_at_tests;
