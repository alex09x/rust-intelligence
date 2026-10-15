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
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Buffer base pointer must be 8-byte aligned for zero-copy RIDX reading",
            ));
        }
        if bytes.len() < std::mem::size_of::<HeaderV2>() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "File too small for RIDX header",
            ));
        }

        let magic: [u8; 4] = bytes[0..4].try_into().unwrap();
        if magic != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid RIDX magic: {:?}", magic),
            ));
        }

        let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let header = match version {
            2 => Header::from_le_v2(&bytes[..std::mem::size_of::<HeaderV2>()]),
            CURRENT_VERSION => {
                if bytes.len() < std::mem::size_of::<Header>() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "File too small for RIDX header",
                    ));
                }
                let mut header_buf = [0u8; std::mem::size_of::<Header>()];
                header_buf.copy_from_slice(&bytes[..std::mem::size_of::<Header>()]);
                let raw_header: Header = unsafe { std::mem::transmute(header_buf) };
                raw_header.from_le()
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Unsupported RIDX version: {}", version),
                ));
            }
        };

        validate_sections(bytes.len(), &header)?;

        Ok(Self { buffer, header })
    }

    #[inline]
    fn bytes(&self) -> &[u8] {
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
        let end = bytes[start..max_end]
            .iter()
            .position(|&b| b == 0)
            .map(|p| start + p)
            .unwrap_or(max_end);

        std::str::from_utf8(&bytes[start..end]).unwrap_or("")
    }

    /// Returns the file path for a given file_id.
    pub fn get_file(&self, file_id: u32) -> &str {
        if file_id >= self.header.files_count {
            return "";
        }
        let offset_in_file_table = (self.header.files_offset as usize)
            + (file_id as usize) * std::mem::size_of::<crate::format::FileEntry>();
        let bytes = self.bytes();
        if offset_in_file_table + std::mem::size_of::<crate::format::FileEntry>() > bytes.len() {
            return "";
        }
        let str_offset = u32::from_le_bytes(
            bytes[offset_in_file_table..offset_in_file_table + 4]
                .try_into()
                .unwrap(),
        );
        self.get_str(str_offset)
    }

    /// Returns the compiled source text hash for a given file_id embedded directly in this index.
    pub fn file_source_hash(&self, file_id: u32) -> u64 {
        if file_id >= self.header.files_count {
            return 0;
        }
        let offset_in_file_table = (self.header.files_offset as usize)
            + (file_id as usize) * std::mem::size_of::<crate::format::FileEntry>();
        let bytes = self.bytes();
        if offset_in_file_table + std::mem::size_of::<crate::format::FileEntry>() > bytes.len() {
            return 0;
        }
        u64::from_le_bytes(
            bytes[offset_in_file_table + 8..offset_in_file_table + 16]
                .try_into()
                .unwrap(),
        )
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

    /// Look up the file_id for a given path substring or exact match.
    pub fn find_file_id(&self, file_path: &str) -> Option<u32> {
        for id in 0..self.header.files_count {
            let f = self.get_file(id);
            if f == file_path || f.ends_with(file_path) || file_path.ends_with(f) {
                return Some(id);
            }
        }
        None
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
    fn defs_by_hash(&self) -> &[HashIndexEntry] {
        let header = self.header();
        let start = header.defs_by_hash_offset as usize;
        let count = header.defs_by_hash_count as usize;
        let len = count * std::mem::size_of::<HashIndexEntry>();
        let bytes = self.bytes();
        if start + len > bytes.len() {
            return &[];
        }
        unsafe {
            std::slice::from_raw_parts(bytes[start..].as_ptr() as *const HashIndexEntry, count)
        }
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
        unsafe {
            std::slice::from_raw_parts(bytes[start..].as_ptr() as *const HashIndexEntry, count)
        }
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
        unsafe {
            std::slice::from_raw_parts(bytes[start..].as_ptr() as *const OwnerIndexEntry, count)
        }
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

    /// Find a definition by its canonical path (e.g. `stdx::panic_context::enter`).
    pub fn find_definition_by_path(&self, path: &str) -> Option<&DefEntry> {
        let hash = fnv1a_64(path.as_bytes());
        let defs_hash = self.defs_by_hash();
        let defs = self.definitions();

        let partition = defs_hash.binary_search_by_key(&hash, |e| e.hash());
        if let Ok(idx) = partition {
            // Check matching hash range
            let mut i = idx;
            while i > 0 && defs_hash[i - 1].hash() == hash {
                i -= 1;
            }
            while i < defs_hash.len() && defs_hash[i].hash() == hash {
                let def = &defs[defs_hash[i].index() as usize];
                if self.get_str(def.path_offset()) == path {
                    return Some(def);
                }
                i += 1;
            }
        }
        None
    }

    /// Go to definition at a given cursor position `(file, line, col)`.
    pub fn find_definition_at(
        &self,
        file: &str,
        line: u32,
        col: u32,
    ) -> Option<ResolvedDefinition<'_>> {
        let file_id = self.find_file_id(file)?;

        // 1. Check references: is the cursor on a symbol reference? Pick innermost span.
        let mut best_ref: Option<(&RefEntry, u64)> = None;
        for r in self.references() {
            let r_span = r.span();
            if r_span.file_id() == file_id && r_span.contains(file_id, line, col) {
                let size = r_span.approximate_size();
                match best_ref {
                    None => best_ref = Some((r, size)),
                    Some((_, best_size)) if size < best_size => best_ref = Some((r, size)),
                    _ => {}
                }
            }
        }

        if let Some((r, _)) = best_ref {
            // If it resolves to a local def_id:
            if r.target_def_id() != u32::MAX {
                let defs = self.definitions();
                if let Some(def) = defs.iter().find(|d| d.def_id() == r.target_def_id()) {
                    let def_span = def.span();
                    return Some(ResolvedDefinition {
                        name: self.get_str(def.name_offset()),
                        path: self.get_str(def.path_offset()),
                        kind: DefKind::from(def.kind),
                        signature: if def.sig_offset() != 0 {
                            Some(self.get_str(def.sig_offset()))
                        } else {
                            None
                        },
                        file: self.get_file(def_span.file_id()),
                        start_line: def_span.start_line(),
                        start_col: def_span.start_col(),
                        end_line: def_span.end_line(),
                        end_col: def_span.end_col(),
                        is_macro_expansion: def.is_macro_expansion(),
                        is_reexport: def.is_reexport(),
                        source_root: None,
                        unit_key: None,
                        reader_idx: None,
                    });
                }
            }
            // Otherwise return target path and location
            let target_path = self.get_str(r.target_path_offset());
            let target_file = self.get_str(r.target_file_offset());
            let target_unit = self.get_str(r.target_unit_offset());
            let target_name = target_path.rsplit("::").next().unwrap_or(target_path);

            return Some(ResolvedDefinition {
                name: target_name,
                path: target_path,
                kind: DefKind::from(r.kind),
                signature: None,
                file: if !target_file.is_empty() {
                    target_file
                } else {
                    "external"
                },
                start_line: r.target_line(),
                start_col: r.target_col(),
                end_line: r.target_line(),
                end_col: r.target_col() + (target_name.len() as u32),
                is_macro_expansion: r.is_macro_expansion(),
                is_reexport: false,
                source_root: None,
                unit_key: if !target_unit.is_empty() {
                    Some(target_unit)
                } else {
                    None
                },
                reader_idx: None,
            });
        }

        // 2. Check if cursor is directly on a definition span (pick innermost span, e.g. struct inside mod)
        let mut best_def: Option<(&DefEntry, u64)> = None;
        for def in self.definitions() {
            let def_span = def.span();
            if def_span.file_id() == file_id && def_span.contains(file_id, line, col) {
                let size = def_span.approximate_size();
                match best_def {
                    None => best_def = Some((def, size)),
                    Some((_, best_size)) if size < best_size => best_def = Some((def, size)),
                    _ => {}
                }
            }
        }

        if let Some((def, _)) = best_def {
            let def_span = def.span();
            return Some(ResolvedDefinition {
                name: self.get_str(def.name_offset()),
                path: self.get_str(def.path_offset()),
                kind: DefKind::from(def.kind),
                signature: if def.sig_offset() != 0 {
                    Some(self.get_str(def.sig_offset()))
                } else {
                    None
                },
                file: self.get_file(def_span.file_id()),
                start_line: def_span.start_line(),
                start_col: def_span.start_col(),
                end_line: def_span.end_line(),
                end_col: def_span.end_col(),
                is_macro_expansion: def.is_macro_expansion(),
                is_reexport: def.is_reexport(),
                source_root: None,
                unit_key: None,
                reader_idx: None,
            });
        }

        None
    }

    /// Find all references to a given symbol path (e.g. `thread::Builder::allow_leak`).
    pub fn find_references(&self, target_path: &str) -> Vec<&RefEntry> {
        let hash = fnv1a_64(target_path.as_bytes());
        let ref_hashes = self.refs_by_target();
        let refs = self.references();
        let mut results = Vec::new();

        if let Ok(idx) = ref_hashes.binary_search_by_key(&hash, |e| e.hash()) {
            let mut i = idx;
            while i > 0 && ref_hashes[i - 1].hash() == hash {
                i -= 1;
            }
            while i < ref_hashes.len() && ref_hashes[i].hash() == hash {
                let r = &refs[ref_hashes[i].index() as usize];
                if self.get_str(r.target_path_offset()) == target_path {
                    results.push(r);
                }
                i += 1;
            }
        }
        results
    }

    /// Finds the innermost TypeEntry at a cursor position `(file, line, col)`.
    pub fn find_innermost_type_entry(&self, file: &str, line: u32, col: u32) -> Option<&TypeEntry> {
        let file_id = self.find_file_id(file)?;
        let types = self.types();
        let key = (file_id, line, col);
        let hi = types.partition_point(|e| {
            let s = e.span();
            (s.file_id(), s.start_line(), s.start_col()) <= key
        });

        let mut best: Option<(&TypeEntry, u64)> = None;
        for entry in types[..hi].iter().rev() {
            let span = entry.span();
            if span.file_id() != file_id {
                break;
            }
            if let Some((_, best_size)) = best {
                let min_size = if line > span.start_line() {
                    (line - span.start_line()) as u64 * 80
                } else {
                    (col.saturating_sub(span.start_col()) as u64).min(80)
                };
                if min_size > best_size {
                    break;
                }
            }
            if span.contains(file_id, line, col) {
                let size = span.approximate_size();
                match best {
                    None => best = Some((entry, size)),
                    Some((_, best_size)) if size <= best_size => best = Some((entry, size)),
                    _ => {}
                }
            }
        }

        best.map(|(e, _)| e)
    }

    /// Retrieves the inferred expression type at a cursor position `(file, line, col)`.
    /// When nested expressions overlap, selects the tightest (innermost) span.
    pub fn type_at(&self, file: &str, line: u32, col: u32) -> Option<&str> {
        self.find_innermost_type_entry(file, line, col)
            .map(|e| self.get_str(e.type_offset()))
    }

    /// Retrieves the inferred expression type and optional canonical owner path at `(file, line, col)`.
    /// Returns `(type_name, canonical_owner_path)`.
    pub fn type_entry_at(&self, file: &str, line: u32, col: u32) -> Option<(&str, Option<&str>)> {
        let entry = self.find_innermost_type_entry(file, line, col)?;
        let ty_name = self.get_str(entry.type_offset());
        let owner_offset = entry.owner_path_offset();
        let owner = if owner_offset != 0 {
            let s = self.get_str(owner_offset);
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        } else {
            None
        };
        Some((ty_name, owner))
    }

    /// Find all definitions belonging to an owner (canonical type path or module path).
    ///
    /// Performs binary search on the 64-bit FNV-1a hash of `owner`, with full collision
    /// verification against the interned canonical owner string.
    pub fn find_definitions_by_owner(&self, owner: &str) -> Vec<&DefEntry> {
        let owner_entries = self.defs_by_owner();
        if owner_entries.is_empty() || owner.is_empty() || owner.starts_with("scope:") {
            return Vec::new();
        }
        let hash = fnv1a_64(owner.as_bytes());
        let defs = self.definitions();
        let mut results = Vec::new();

        if let Ok(idx) = owner_entries.binary_search_by_key(&hash, |e| e.hash()) {
            let mut start = idx;
            while start > 0 && owner_entries[start - 1].hash() == hash {
                start -= 1;
            }
            let mut i = start;
            while i < owner_entries.len() && owner_entries[i].hash() == hash {
                let entry = &owner_entries[i];
                if self.get_str(entry.owner_path_offset()) == owner {
                    let def_idx = entry.def_index() as usize;
                    if def_idx < defs.len() {
                        results.push(&defs[def_idx]);
                    }
                }
                i += 1;
            }
        }

        results
    }

    /// Find all canonical owners associated with a definition index.
    pub fn find_owners_by_def_index(&self, def_index: u32) -> Vec<&str> {
        let mut owners = Vec::new();
        for entry in self.defs_by_owner() {
            if entry.def_index() == def_index {
                let s = self.get_str(entry.owner_path_offset());
                if !s.is_empty() && !s.starts_with("scope:") && !owners.contains(&s) {
                    owners.push(s);
                }
            }
        }
        owners
    }

    /// Returns the lexical enclosing scope coordinates `(start_line, start_col, end_line, end_col)`
    /// for a definition index if recorded.
    pub fn type_param_scope(&self, def_index: u32) -> Option<(u32, u32, u32, u32)> {
        for entry in self.defs_by_owner() {
            if entry.def_index() == def_index {
                let s = self.get_str(entry.owner_path_offset());
                if let Some(rest) = s.strip_prefix("scope:") {
                    let mut parts = rest.split(':');
                    if let (Some(sl), Some(sc), Some(el), Some(ec)) = (
                        parts.next().and_then(|p| p.parse().ok()),
                        parts.next().and_then(|p| p.parse().ok()),
                        parts.next().and_then(|p| p.parse().ok()),
                        parts.next().and_then(|p| p.parse().ok()),
                    ) {
                        return Some((sl, sc, el, ec));
                    }
                }
            }
        }
        None
    }

    /// Finds a generic type parameter (`DefKind::TyParam`) in the specified file matching `name`
    /// whose enclosing item span contains `(line, col)`.
    pub fn find_type_param_in_scope(
        &self,
        file: &str,
        name: &str,
        line: u32,
        col: u32,
    ) -> Option<(&DefEntry, u32)> {
        let file_id = self.find_file_id(file)?;
        let mut best: Option<(&DefEntry, u32, u64)> = None;

        for (idx, def) in self.definitions().iter().enumerate() {
            if def.kind != DefKind::TyParam as u8 {
                continue;
            }
            if self.get_str(def.name_offset()) != name {
                continue;
            }
            let span = def.span();
            if span.file_id() != file_id {
                continue;
            }

            // 1. Check explicit enclosing scope recorded in owners
            let mut matched_scope = None;
            if let Some((sl, sc, el, ec)) = self.type_param_scope(idx as u32) {
                let in_range = (line > sl || (line == sl && col >= sc))
                    && (line < el || (line == el && col <= ec));
                if in_range {
                    let size = ((el.saturating_sub(sl)) as u64) * 10_000
                        + ((ec.saturating_sub(sc)) as u64);
                    matched_scope = Some(size);
                }
            }

            // 2. Fallback: check parent definition by path or def span
            let (in_scope, size) = if let Some(scope_size) = matched_scope {
                (true, scope_size)
            } else {
                let path = self.get_str(def.path_offset());
                if let Some((parent_path, _)) = path.rsplit_once("::") {
                    if let Some(parent_def) = self.find_definition_by_path(parent_path) {
                        let p_span = parent_def.span();
                        let in_range = p_span.file_id() == file_id
                            && (line > p_span.start_line()
                                || (line == p_span.start_line() && col >= p_span.start_col()))
                            && (line < p_span.end_line()
                                || (line == p_span.end_line() && col <= p_span.end_col()));
                        (in_range, p_span.approximate_size())
                    } else {
                        let in_range = line >= span.start_line() && line <= span.end_line();
                        (in_range, span.approximate_size())
                    }
                } else {
                    let in_range = line >= span.start_line() && line <= span.end_line();
                    (in_range, span.approximate_size())
                }
            };

            if in_scope {
                match best {
                    None => best = Some((def, idx as u32, size)),
                    Some((_, _, best_size)) if size < best_size => {
                        best = Some((def, idx as u32, size))
                    }
                    _ => {}
                }
            }
        }

        best.map(|(d, idx, _)| (d, idx))
    }

    /// Look up all direct Deref or supertrait targets for a canonical type path.
    pub fn find_deref_targets(&self, source: &str) -> Vec<&str> {
        let mut targets = Vec::new();
        let entries = self.deref_entries();
        if entries.is_empty() || source.is_empty() {
            return targets;
        }
        let hash = fnv1a_64(source.as_bytes());
        if let Ok(idx) = entries.binary_search_by_key(&hash, |e| e.source_type_hash()) {
            let mut start = idx;
            while start > 0 && entries[start - 1].source_type_hash() == hash {
                start -= 1;
            }
            let mut i = start;
            while i < entries.len() && entries[i].source_type_hash() == hash {
                let entry = &entries[i];
                if self.get_str(entry.source_type_offset()) == source {
                    targets.push(self.get_str(entry.target_type_offset()));
                }
                i += 1;
            }
        }
        targets
    }

    /// Look up the first direct Deref target for a canonical type path.
    pub fn find_deref_target(&self, source: &str) -> Option<&str> {
        self.find_deref_targets(source).into_iter().next()
    }

    /// Resolves the autoderef/supertrait chain starting from `source` up to `max_depth` steps.
    ///
    /// Returns the sequence of target type canonical paths (excluding `source`).
    /// Handles multiple supertraits and cycles gracefully via breadth-first search and visited tracking.
    pub fn resolve_deref_chain(&self, source: &str, max_depth: usize) -> Vec<&str> {
        let mut chain = Vec::new();
        let mut queue = std::collections::VecDeque::new();
        let mut visited = std::collections::HashSet::new();
        visited.insert(source);
        queue.push_back((source, 0));

        while let Some((current, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }
            for target in self.find_deref_targets(current) {
                if !target.is_empty() && visited.insert(target) {
                    chain.push(target);
                    queue.push_back((target, depth + 1));
                }
            }
        }

        chain
    }

    /// Fast symbol search across definitions matching a query string.
    ///
    /// For ASCII queries, this avoids per-entry heap allocations by lowercasing the query once
    /// and performing case-insensitive byte matching directly over string pool slices.
    pub fn search_symbols(&self, query: &str) -> Vec<&DefEntry> {
        if query.is_ascii() {
            let q_lower = query.to_ascii_lowercase();
            self.search_symbols_ascii(q_lower.as_bytes())
        } else {
            let mut matches = Vec::new();
            let q = query.to_lowercase();
            for def in self.definitions() {
                if def.kind == DefKind::TyParam as u8 {
                    continue;
                }
                let name = self.get_str(def.name_offset());
                let path = self.get_str(def.path_offset());
                if name.to_lowercase().contains(&q) || path.to_lowercase().contains(&q) {
                    matches.push(def);
                }
            }
            matches
        }
    }

    /// Fast ASCII case-insensitive symbol search across definitions using a pre-lowercased query slice.
    pub fn search_symbols_ascii(&self, needle_lower: &[u8]) -> Vec<&DefEntry> {
        let mut matches = Vec::new();
        for def in self.definitions() {
            if def.kind == DefKind::TyParam as u8 {
                continue;
            }
            let name = self.get_str(def.name_offset());
            let path = self.get_str(def.path_offset());
            if contains_ignore_case_ascii(name, needle_lower)
                || contains_ignore_case_ascii(path, needle_lower)
            {
                matches.push(def);
            }
        }
        matches
    }

    /// Iterates over matching definitions without heap allocation, stopping when `cb` returns false
    /// or when `budget` is exhausted.
    pub fn for_each_symbol_ascii<F>(&self, needle_lower: &[u8], budget: &mut usize, cb: F) -> bool
    where
        F: FnMut(&DefEntry) -> bool,
    {
        let mut record_budget = usize::MAX;
        self.for_each_symbol_ascii_budgeted(needle_lower, budget, &mut record_budget, cb)
    }

    /// Iterates over matching definitions without heap allocation, stopping when `cb` returns false,
    /// when match `budget` is exhausted, or when `record_budget` of inspected definitions is reached.
    pub fn for_each_symbol_ascii_budgeted<F>(
        &self,
        needle_lower: &[u8],
        match_budget: &mut usize,
        record_budget: &mut usize,
        mut cb: F,
    ) -> bool
    where
        F: FnMut(&DefEntry) -> bool,
    {
        for def in self.definitions() {
            if *record_budget == 0 {
                return false;
            }
            *record_budget -= 1;
            if def.kind == DefKind::TyParam as u8 {
                continue;
            }
            let name = self.get_str(def.name_offset());
            let path = self.get_str(def.path_offset());
            if contains_ignore_case_ascii(name, needle_lower)
                || contains_ignore_case_ascii(path, needle_lower)
            {
                if *match_budget == 0 {
                    return false;
                }
                *match_budget -= 1;
                if !cb(def) {
                    return false;
                }
            }
        }
        true
    }
}

/// Case-insensitive ASCII substring search without heap allocations.
/// Returns true if `haystack` contains `needle_lower` case-insensitively.
/// `needle_lower` MUST already be in ASCII lowercase.
#[inline]
pub(crate) fn contains_ignore_case_ascii(haystack: &str, needle_lower: &[u8]) -> bool {
    if needle_lower.is_empty() {
        return true;
    }
    let h = haystack.as_bytes();
    let n = needle_lower.len();
    if h.len() < n {
        return false;
    }
    let q0 = needle_lower[0];
    let q0_upper = q0.to_ascii_uppercase();
    let rest = &needle_lower[1..];
    for i in 0..=(h.len() - n) {
        let b = h[i];
        if (b == q0 || b == q0_upper)
            && h[i + 1..i + n]
                .iter()
                .zip(rest)
                .all(|(&hb, &qb)| hb.to_ascii_lowercase() == qb)
        {
            return true;
        }
    }
    false
}

fn validate_sections(buf_len: usize, header: &Header) -> io::Result<()> {
    // 1. String pool alignment and bounds
    if (header.string_pool_offset as usize) % 8 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "String pool offset is not 8-byte aligned",
        ));
    }
    let pool_start = header.string_pool_offset as usize;
    let pool_len = header.string_pool_len as usize;
    let pool_end = pool_start
        .checked_add(pool_len)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "String pool length overflow"))?;
    if pool_end > buf_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "String pool extends beyond file bounds",
        ));
    }

    if (header.crate_name_offset as usize) >= pool_len
        || (header.crate_version_offset as usize) >= pool_len
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Crate metadata string offsets exceed string pool bounds",
        ));
    }

    // 2. Files table
    check_section(
        buf_len,
        header.files_offset,
        header.files_count,
        std::mem::size_of::<FileEntry>(),
        std::mem::align_of::<FileEntry>(),
        "Files table",
    )?;

    // 3. Definitions table
    check_section(
        buf_len,
        header.defs_offset,
        header.defs_count,
        std::mem::size_of::<DefEntry>(),
        4,
        "Definitions table",
    )?;

    // 4. Definitions hash index
    check_section(
        buf_len,
        header.defs_by_hash_offset,
        header.defs_by_hash_count,
        std::mem::size_of::<HashIndexEntry>(),
        8,
        "Definitions hash index",
    )?;

    // 5. References table
    check_section(
        buf_len,
        header.refs_offset,
        header.refs_count,
        std::mem::size_of::<RefEntry>(),
        4,
        "References table",
    )?;

    // 6. References target hash index
    check_section(
        buf_len,
        header.refs_by_target_offset,
        header.refs_by_target_count,
        std::mem::size_of::<HashIndexEntry>(),
        8,
        "References target hash index",
    )?;

    // 7. Types table
    check_section(
        buf_len,
        header.types_offset,
        header.types_count,
        std::mem::size_of::<TypeEntry>(),
        4,
        "Types table",
    )?;

    // 8. Definitions by owner table
    check_section(
        buf_len,
        header.defs_by_owner_offset,
        header.defs_by_owner_count,
        std::mem::size_of::<OwnerIndexEntry>(),
        8,
        "Definitions by owner table",
    )?;

    // 9. Derefs table
    check_section(
        buf_len,
        header.derefs_offset,
        header.derefs_count,
        std::mem::size_of::<DerefEntry>(),
        8,
        "Derefs table",
    )?;

    Ok(())
}

fn check_section(
    buf_len: usize,
    offset: u64,
    count: u32,
    elem_size: usize,
    required_align: usize,
    section_name: &str,
) -> io::Result<()> {
    let off = offset as usize;
    if off % required_align != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{section_name} offset {offset} is not aligned to {required_align} bytes"),
        ));
    }
    if count == 0 {
        let max_empty_offset = buf_len.div_ceil(8) * 8;
        if off > max_empty_offset {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{section_name} offset {offset} extends beyond file bounds ({max_empty_offset})"),
            ));
        }
        return Ok(());
    }
    let total_bytes = (count as usize).checked_mul(elem_size).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{section_name} size calculation overflow"),
        )
    })?;
    let end = off.checked_add(total_bytes).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{section_name} end offset overflow"),
        )
    })?;
    if end > buf_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{section_name} extends beyond file bounds ({end} > {buf_len})"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolved_definition() {
        let def = ResolvedDefinition {
            name: "test_fn",
            path: "crate::test_fn",
            kind: DefKind::Fn,
            signature: Some("pub fn test_fn()"),
            file: "src/lib.rs",
            start_line: 1,
            start_col: 1,
            end_line: 3,
            end_col: 2,
            is_macro_expansion: false,
            is_reexport: false,
            source_root: Some(Path::new("/workspace")),
            unit_key: Some("unit1"),
            reader_idx: None,
        };

        assert_eq!(def.resolved_path(), Path::new("/workspace/src/lib.rs"));

        let abs_def = ResolvedDefinition {
            file: "/absolute/path/src/lib.rs",
            source_root: Some(Path::new("/other")),
            ..def
        };
        assert_eq!(
            abs_def.resolved_path(),
            Path::new("/absolute/path/src/lib.rs")
        );

        let no_root_def = ResolvedDefinition {
            source_root: None,
            ..def
        };
        assert_eq!(no_root_def.resolved_path(), Path::new("src/lib.rs"));
    }

    #[test]
    fn test_reader_validation_errors() {
        // 1. Buffer too small
        let small_buf = vec![0u8; 10];
        let err = IndexReader::from_buffer(small_buf).unwrap_err();
        assert!(err.to_string().contains("File too small"));

        // 2. Unaligned buffer
        #[repr(align(8))]
        struct AlignedBuf([u8; 1024]);
        let aligned = AlignedBuf([0u8; 1024]);
        let unaligned_slice = &aligned.0[1..512];
        let err = IndexReader::from_buffer(unaligned_slice).unwrap_err();
        assert!(err.to_string().contains("8-byte aligned"));

        // 3. Invalid magic
        let mut bad_magic = vec![0u8; std::mem::size_of::<Header>() + 100];
        bad_magic[0..4].copy_from_slice(b"BAD!");
        let err = IndexReader::from_buffer(bad_magic).unwrap_err();
        assert!(err.to_string().contains("Invalid RIDX magic"));

        // 4. Unsupported version
        let mut bad_version = vec![0u8; std::mem::size_of::<Header>() + 100];
        bad_version[0..4].copy_from_slice(&MAGIC);
        bad_version[4..8].copy_from_slice(&999u32.to_le_bytes());
        let err = IndexReader::from_buffer(bad_version).unwrap_err();
        assert!(err.to_string().contains("Unsupported RIDX version"));
    }

    #[test]
    fn test_check_section_errors() {
        // Unaligned offset
        let err = check_section(1000, 3, 10, 4, 8, "TestSec").unwrap_err();
        assert!(err.to_string().contains("is not aligned to 8 bytes"));

        // Size calculation overflow
        let err = check_section(1000, 8, u32::MAX, usize::MAX / 2, 8, "TestSec").unwrap_err();
        assert!(err.to_string().contains("overflow"));

        // Extends beyond file bounds
        let err = check_section(50, 16, 10, 8, 8, "TestSec").unwrap_err();
        assert!(err.to_string().contains("extends beyond file bounds"));

        // Empty section offset beyond file bounds
        let err = check_section(100, 200, 0, 8, 8, "TestSec").unwrap_err();
        assert!(err.to_string().contains("extends beyond file bounds"));

        // End offset overflow
        let err =
            check_section(1000, ((usize::MAX - 15) & !7) as u64, 2, 20, 8, "TestSec").unwrap_err();
        assert!(err.to_string().contains("overflow"));
    }

    #[test]
    fn test_reader_accessors_out_of_bounds() {
        let temp_dir = std::env::temp_dir().join(format!("ridx_rdr_bounds_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let idx_path = temp_dir.join("test.ridx");

        let mut writer = crate::writer::IndexWriter::new("rdr_crate", "0.1.0");
        writer.add_definition(crate::writer::RawDefinition {
            def_id: 1,
            kind: crate::format::DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "fn1".to_string(),
            path: "rdr_crate::fn1".to_string(),
            signature: None,
            file: "src/lib.rs".to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 10,
        });
        let mut f = std::fs::File::create(&idx_path).unwrap();
        writer.write_to(&mut f).unwrap();
        drop(f);

        let reader = IndexReader::open(&idx_path).unwrap();

        // 1. get_str with invalid offset
        assert_eq!(reader.get_str(u32::MAX), "");
        assert_eq!(
            reader.get_str((reader.header.string_pool_len as u32).saturating_add(10)),
            ""
        );

        // 2. get_file with invalid file_id
        assert_eq!(reader.get_file(u32::MAX), "");

        // 3. file_source_hash with invalid file_id
        assert_eq!(reader.file_source_hash(u32::MAX), 0);

        // 4. find_definition_by_path nonexistent
        assert!(reader.find_definition_by_path("nonexistent").is_none());

        // 5. find_definition_at outside of file
        assert!(reader.find_definition_at("nonexistent.rs", 1, 1).is_none());

        // Truncated buffer bounds checks (lines 165, 185, 204)
        let dummy_hdr = Header {
            string_pool_offset: 1000,
            string_pool_len: 100,
            files_offset: 2000,
            files_count: 5,
            ..reader.header
        };
        let dummy_reader = IndexReader {
            buffer: vec![0u8; 100],
            header: dummy_hdr,
        };
        assert_eq!(dummy_reader.get_str(5), "");
        assert_eq!(dummy_reader.get_file(0), "");
        assert_eq!(dummy_reader.file_source_hash(0), 0);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_reader_overlapping_spans_and_hash_collisions() {
        let temp_dir =
            std::env::temp_dir().join(format!("ridx_rdr_overlap_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let idx_path = temp_dir.join("overlap.ridx");

        let mut writer = crate::writer::IndexWriter::new("overlap_crate", "0.1.0");
        // Outer definition
        writer.add_definition(crate::writer::RawDefinition {
            def_id: 1,
            kind: crate::format::DefKind::Mod,
            visibility: 0,
            is_macro_expansion: false,
            name: "outer_mod".to_string(),
            path: "overlap_crate::outer_mod".to_string(),
            signature: None,
            file: "src/lib.rs".to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 50,
            end_col: 1,
        });
        // Inner definition (smaller span)
        writer.add_definition(crate::writer::RawDefinition {
            def_id: 2,
            kind: crate::format::DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "inner_fn".to_string(),
            path: "overlap_crate::outer_mod::inner_fn".to_string(),
            signature: Some("fn() -> i32".to_string()),
            file: "src/lib.rs".to_string(),
            start_line: 10,
            start_col: 5,
            end_line: 15,
            end_col: 6,
        });

        // Overlapping references: outer ref and inner ref
        writer.add_reference(crate::writer::RawReference {
            file: "src/lib.rs".to_string(),
            start_line: 20,
            start_col: 1,
            end_line: 20,
            end_col: 30,
            target_def_id: Some(1),
            target_path: "overlap_crate::outer_mod".to_string(),
            target_unit: None,
            target_file: Some("src/lib.rs".to_string()),
            target_line: 1,
            target_col: 1,
            kind: crate::format::DefKind::Mod,
            is_write: false,
            is_macro_expansion: false,
        });
        writer.add_reference(crate::writer::RawReference {
            file: "src/lib.rs".to_string(),
            start_line: 20,
            start_col: 10,
            end_line: 20,
            end_col: 20,
            target_def_id: Some(2),
            target_path: "overlap_crate::outer_mod::inner_fn".to_string(),
            target_unit: None,
            target_file: Some("src/lib.rs".to_string()),
            target_line: 10,
            target_col: 5,
            kind: crate::format::DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });
        // External reference (target_def_id == u32::MAX)
        writer.add_reference(crate::writer::RawReference {
            file: "src/lib.rs".to_string(),
            start_line: 30,
            start_col: 5,
            end_line: 30,
            end_col: 15,
            target_def_id: None,
            target_path: "ext_crate::ext_fn".to_string(),
            target_unit: Some("ext_crate:1.0.0".to_string()),
            target_file: Some("ext.rs".to_string()),
            target_line: 5,
            target_col: 1,
            kind: crate::format::DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });

        // Overlapping types
        writer.add_type(crate::writer::RawType {
            file: "src/lib.rs".to_string(),
            start_line: 25,
            start_col: 1,
            end_line: 25,
            end_col: 30,
            type_name: "OuterType".to_string(),
        });
        writer.add_type(crate::writer::RawType {
            file: "src/lib.rs".to_string(),
            start_line: 25,
            start_col: 5,
            end_line: 25,
            end_col: 15,
            type_name: "InnerType".to_string(),
        });

        // Missing def_id target in reference (line 386)
        writer.add_reference(crate::writer::RawReference {
            file: "src/lib.rs".to_string(),
            start_line: 40,
            start_col: 1,
            end_line: 40,
            end_col: 10,
            target_def_id: Some(9999), // does not exist in defs!
            target_path: "unresolved::missing".to_string(),
            target_unit: None,
            target_file: Some("src/missing.rs".to_string()),
            target_line: 1,
            target_col: 1,
            kind: crate::format::DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });

        // Add inner items FIRST then outer items SECOND on src/other.rs to hit `_ => {}` (lines 357, 427, 493)
        // 1. Definition: inner first, outer second
        writer.add_definition(crate::writer::RawDefinition {
            def_id: 3,
            kind: crate::format::DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "inner_def".to_string(),
            path: "overlap_crate::inner_def".to_string(),
            signature: None,
            file: "src/other.rs".to_string(),
            start_line: 5,
            start_col: 5,
            end_line: 5,
            end_col: 10,
        });
        writer.add_definition(crate::writer::RawDefinition {
            def_id: 4,
            kind: crate::format::DefKind::Mod,
            visibility: 0,
            is_macro_expansion: false,
            name: "outer_mod2".to_string(),
            path: "overlap_crate::outer_mod2".to_string(),
            signature: None,
            file: "src/other.rs".to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 10,
            end_col: 1,
        });

        // 2. Reference: inner first, outer second
        writer.add_reference(crate::writer::RawReference {
            file: "src/other.rs".to_string(),
            start_line: 15,
            start_col: 5,
            end_line: 15,
            end_col: 10,
            target_def_id: Some(3),
            target_path: "overlap_crate::inner_def".to_string(),
            target_unit: None,
            target_file: Some("src/other.rs".to_string()),
            target_line: 5,
            target_col: 5,
            kind: crate::format::DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });
        writer.add_reference(crate::writer::RawReference {
            file: "src/other.rs".to_string(),
            start_line: 15,
            start_col: 1,
            end_line: 15,
            end_col: 20,
            target_def_id: Some(4),
            target_path: "overlap_crate::outer_mod2".to_string(),
            target_unit: None,
            target_file: Some("src/other.rs".to_string()),
            target_line: 1,
            target_col: 1,
            kind: crate::format::DefKind::Mod,
            is_write: false,
            is_macro_expansion: false,
        });

        // 3. Type: inner first, outer second
        writer.add_type(crate::writer::RawType {
            file: "src/other.rs".to_string(),
            start_line: 20,
            start_col: 5,
            end_line: 20,
            end_col: 10,
            type_name: "InnerOther".to_string(),
        });
        writer.add_type(crate::writer::RawType {
            file: "src/other.rs".to_string(),
            start_line: 20,
            start_col: 1,
            end_line: 20,
            end_col: 20,
            type_name: "OuterOther".to_string(),
        });

        let mut f = std::fs::File::create(&idx_path).unwrap();
        writer.write_to(&mut f).unwrap();
        drop(f);

        let reader = IndexReader::open(&idx_path).unwrap();

        // 1. Definition cursor picking innermost def
        let def = reader.find_definition_at("src/lib.rs", 12, 6).unwrap();
        assert_eq!(def.name, "inner_fn");
        assert_eq!(def.signature, Some("fn() -> i32"));

        let def_outer = reader.find_definition_at("src/lib.rs", 2, 1).unwrap();
        assert_eq!(def_outer.name, "outer_mod");
        assert_eq!(def_outer.signature, None);

        // 2. Reference cursor picking innermost ref with local def_id
        let ref_inner = reader.find_definition_at("src/lib.rs", 20, 15).unwrap();
        assert_eq!(ref_inner.name, "inner_fn");

        // Reference cursor with external def
        let ref_ext = reader.find_definition_at("src/lib.rs", 30, 8).unwrap();
        assert_eq!(ref_ext.path, "ext_crate::ext_fn");
        assert_eq!(ref_ext.file, "ext.rs");

        // Reference cursor with nonexistent target_def_id (line 386)
        let ref_unresolved = reader.find_definition_at("src/lib.rs", 40, 5).unwrap();
        assert_eq!(ref_unresolved.path, "unresolved::missing");

        // 3. Type at cursor picking innermost type
        assert_eq!(reader.type_at("src/lib.rs", 25, 10), Some("InnerType"));
        assert_eq!(reader.type_at("src/lib.rs", 25, 2), Some("OuterType"));
        assert_eq!(reader.type_at("src/lib.rs", 99, 99), None);
        assert_eq!(reader.type_at("missing.rs", 1, 1), None);

        // 4. Reverse ordering tests (lines 357, 427, 493)
        assert_eq!(
            reader
                .find_definition_at("src/other.rs", 5, 6)
                .unwrap()
                .name,
            "inner_def"
        );
        assert_eq!(
            reader
                .find_definition_at("src/other.rs", 15, 6)
                .unwrap()
                .name,
            "inner_def"
        );
        assert_eq!(reader.type_at("src/other.rs", 20, 6), Some("InnerOther"));

        // 5. find_references
        let refs = reader.find_references("overlap_crate::outer_mod::inner_fn");
        assert_eq!(refs.len(), 1);
        let missing_refs = reader.find_references("missing::path");
        assert!(missing_refs.is_empty());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_validate_sections_comprehensive() {
        // String pool offset not aligned
        let mut hdr = Header {
            magic: crate::format::MAGIC,
            version: crate::format::CURRENT_FORMAT_VERSION,
            crate_name_offset: 0,
            crate_version_offset: 0,
            string_pool_offset: 7, // not aligned to 8
            string_pool_len: 16,
            files_offset: 16,
            files_count: 0,
            defs_offset: 16,
            defs_count: 0,
            defs_by_hash_offset: 16,
            defs_by_hash_count: 0,
            refs_offset: 16,
            refs_count: 0,
            refs_by_target_offset: 16,
            refs_by_target_count: 0,
            types_offset: 16,
            types_count: 0,
            unit_key_offset: 0,
            generation_offset: 0,
            _pad2: 0,
            _pad3: 0,
            _pad4: 0,
            _pad5: 0,
            defs_by_owner_offset: 16,
            defs_by_owner_count: 0,
            _pad6: 0,
            derefs_offset: 16,
            derefs_count: 0,
            _pad7: 0,
        };
        assert!(validate_sections(1000, &hdr).is_err());

        // String pool length overflow
        hdr.string_pool_offset = 16;
        hdr.string_pool_len = u64::MAX;
        assert!(validate_sections(1000, &hdr).is_err());

        // String pool beyond file bounds
        hdr.string_pool_len = 2000;
        assert!(validate_sections(1000, &hdr).is_err());

        // Crate name offset beyond pool
        hdr.string_pool_len = 100;
        hdr.crate_name_offset = 200;
        assert!(validate_sections(1000, &hdr).is_err());

        hdr.crate_name_offset = 0;
        hdr.crate_version_offset = 200;
        assert!(validate_sections(1000, &hdr).is_err());

        // Test section error paths (lines 563, 573, 583, 593, 603)
        hdr.crate_version_offset = 0;
        hdr.defs_offset = 3; // unaligned
        assert!(validate_sections(1000, &hdr).is_err());
        hdr.defs_offset = 16;
        hdr.defs_by_hash_offset = 3;
        assert!(validate_sections(1000, &hdr).is_err());
        hdr.defs_by_hash_offset = 16;
        hdr.refs_offset = 3;
        assert!(validate_sections(1000, &hdr).is_err());
        hdr.refs_offset = 16;
        hdr.refs_by_target_offset = 3;
        assert!(validate_sections(1000, &hdr).is_err());
        hdr.refs_by_target_offset = 16;
        hdr.types_offset = 3;
        assert!(validate_sections(1000, &hdr).is_err());

        // Open non-existent file
        assert!(IndexReader::open("/nonexistent/ridx/file.ridx").is_err());
    }

    #[test]
    fn test_reader_corrupted_offsets_and_bounds() {
        let temp_dir =
            std::env::temp_dir().join(format!("ridx_rdr_corrupt_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let idx_path = temp_dir.join("test.ridx");

        let mut writer = crate::writer::IndexWriter::new("corrupt_crate", "0.1.0");
        let mut f = std::fs::File::create(&idx_path).unwrap();
        writer.write_to(&mut f).unwrap();
        drop(f);

        let mut reader = IndexReader::open(&idx_path).unwrap();
        // Modify header offsets to exceed file bounds:
        reader.header.defs_offset = 1_000_000;
        reader.header.defs_count = 10;
        assert_eq!(reader.definitions().len(), 0);

        reader.header.defs_by_hash_offset = 1_000_000;
        reader.header.defs_by_hash_count = 10;
        assert_eq!(reader.defs_by_hash().len(), 0);

        reader.header.refs_offset = 1_000_000;
        reader.header.refs_count = 10;
        assert_eq!(reader.references().len(), 0);

        reader.header.refs_by_target_offset = 1_000_000;
        reader.header.refs_by_target_count = 10;
        assert_eq!(reader.refs_by_target().len(), 0);

        reader.header.types_offset = 1_000_000;
        reader.header.types_count = 10;
        assert_eq!(reader.types().len(), 0);

        let _ = std::fs::remove_dir_all(&temp_dir);

        // ResolvedDefinition resolved_path with None source_root (lines 45-47)
        let resolved = ResolvedDefinition {
            name: "test",
            path: "test::fn",
            kind: DefKind::Fn,
            signature: None,
            file: "src/lib.rs",
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 10,
            is_macro_expansion: false,
            is_reexport: false,
            source_root: None,
            unit_key: None,
            reader_idx: None,
        };
        assert_eq!(
            resolved.resolved_path(),
            std::path::PathBuf::from("src/lib.rs")
        );
    }

    #[test]
    fn test_type_at_binary_search_vs_linear_property() {
        let temp_dir =
            std::env::temp_dir().join(format!("ridx_test_types_prop_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let idx_path = temp_dir.join("nested_types.ridx");
        let mut writer = crate::writer::IndexWriter::new("test_types", "0.1.0");

        writer.add_definition(crate::writer::RawDefinition {
            def_id: 1,
            kind: crate::format::DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "dummy".to_string(),
            path: "test::dummy".to_string(),
            signature: None,
            file: "src/lib.rs".to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 5,
        });

        // File 1: src/lib.rs
        // Outer expression: foo(...) spanning line 10, col 5 to line 12, col 20 (size: 2*80 + 20 = 180)
        writer.add_type(crate::writer::RawType {
            file: "src/lib.rs".to_string(),
            start_line: 10,
            start_col: 5,
            end_line: 12,
            end_col: 20,
            type_name: "TypeFoo".to_string(),
        });
        // Middle expression: bar(...) spanning line 11, col 8 to line 11, col 30 (size: 22)
        writer.add_type(crate::writer::RawType {
            file: "src/lib.rs".to_string(),
            start_line: 11,
            start_col: 8,
            end_line: 11,
            end_col: 30,
            type_name: "TypeBar".to_string(),
        });
        // Inner expression: baz(...) spanning line 11, col 12 to line 11, col 25 (size: 13)
        writer.add_type(crate::writer::RawType {
            file: "src/lib.rs".to_string(),
            start_line: 11,
            start_col: 12,
            end_line: 11,
            end_col: 25,
            type_name: "TypeBaz".to_string(),
        });
        // Innermost expression: x spanning line 11, col 16 to line 11, col 17 (size: 1)
        writer.add_type(crate::writer::RawType {
            file: "src/lib.rs".to_string(),
            start_line: 11,
            start_col: 16,
            end_line: 11,
            end_col: 17,
            type_name: "TypeX".to_string(),
        });
        // Sibling expression: y spanning line 11, col 20 to line 11, col 22 (size: 2)
        writer.add_type(crate::writer::RawType {
            file: "src/lib.rs".to_string(),
            start_line: 11,
            start_col: 20,
            end_line: 11,
            end_col: 22,
            type_name: "TypeY".to_string(),
        });

        // Same start pos expressions: a.b.c()
        writer.add_type(crate::writer::RawType {
            file: "src/lib.rs".to_string(),
            start_line: 20,
            start_col: 5,
            end_line: 20,
            end_col: 6,
            type_name: "TypeA".to_string(),
        });
        writer.add_type(crate::writer::RawType {
            file: "src/lib.rs".to_string(),
            start_line: 20,
            start_col: 5,
            end_line: 20,
            end_col: 8,
            type_name: "TypeAB".to_string(),
        });
        writer.add_type(crate::writer::RawType {
            file: "src/lib.rs".to_string(),
            start_line: 20,
            start_col: 5,
            end_line: 20,
            end_col: 12,
            type_name: "TypeABC".to_string(),
        });

        // File 2: src/other.rs with adjacent spans
        writer.add_type(crate::writer::RawType {
            file: "src/other.rs".to_string(),
            start_line: 5,
            start_col: 0,
            end_line: 5,
            end_col: 10,
            type_name: "OtherSpan1".to_string(),
        });
        writer.add_type(crate::writer::RawType {
            file: "src/other.rs".to_string(),
            start_line: 5,
            start_col: 10,
            end_line: 5,
            end_col: 20,
            type_name: "OtherSpan2".to_string(),
        });

        let mut f = std::fs::File::create(&idx_path).unwrap();
        writer.write_to(&mut f).unwrap();
        drop(f);

        let reader = IndexReader::open(&idx_path).unwrap();

        // Linear reference implementation (old behavior)
        let type_at_linear = |file: &str, line: u32, col: u32| -> Option<String> {
            let file_id = reader.find_file_id(file)?;
            let mut best: Option<(&TypeEntry, u64)> = None;
            for entry in reader.types() {
                let span = entry.span();
                if span.file_id() == file_id && span.contains(file_id, line, col) {
                    let size = span.approximate_size();
                    match best {
                        None => best = Some((entry, size)),
                        Some((_, best_size)) if size < best_size => best = Some((entry, size)),
                        _ => {}
                    }
                }
            }
            best.map(|(e, _)| reader.get_str(e.type_offset()).to_string())
        };

        // Grid check across all coordinates in both files and missing file
        for file in &["src/lib.rs", "src/other.rs", "missing.rs"] {
            for line in 0..30 {
                for col in 0..40 {
                    let expected = type_at_linear(file, line, col);
                    let actual = reader.type_at(file, line, col).map(|s| s.to_string());
                    assert_eq!(
                        actual, expected,
                        "Mismatch at file={file}, line={line}, col={col}: actual={actual:?}, expected={expected:?}"
                    );
                }
            }
        }

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_type_at_large_scale_benchmark() {
        let temp_dir =
            std::env::temp_dir().join(format!("ridx_test_types_scale_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let idx_path = temp_dir.join("large_types.ridx");
        let mut writer = crate::writer::IndexWriter::new("test_scale", "0.1.0");

        writer.add_definition(crate::writer::RawDefinition {
            def_id: 1,
            kind: crate::format::DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "main".to_string(),
            path: "test::main".to_string(),
            signature: None,
            file: "src/main.rs".to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 5,
        });

        // Generate 100,000 synthetic types across multiple files
        let num_files = 50;
        let types_per_file = 2_000;
        for f in 0..num_files {
            let file_name = format!("src/file_{}.rs", f);
            for i in 0..types_per_file {
                let line = (i * 2 + 1) as u32;
                writer.add_type(crate::writer::RawType {
                    file: file_name.clone(),
                    start_line: line,
                    start_col: 5,
                    end_line: line,
                    end_col: 25,
                    type_name: format!("Type_{}_{}", f, i),
                });
            }
        }

        let mut f = std::fs::File::create(&idx_path).unwrap();
        writer.write_to(&mut f).unwrap();
        drop(f);

        let reader = IndexReader::open(&idx_path).unwrap();
        assert_eq!(reader.types().len(), 100_000);

        // Benchmark queries across 100 sample locations
        let samples: Vec<(String, u32, u32)> = (0..100)
            .map(|i| {
                let file = format!("src/file_{}.rs", (i * 7) % num_files);
                let line = ((i * 13) % types_per_file) as u32 * 2 + 1;
                (file, line, 10)
            })
            .collect();

        // 1. Time binary search (new implementation)
        let t0 = std::time::Instant::now();
        for (file, line, col) in &samples {
            let res = reader.type_at(file, *line, *col);
            assert!(res.is_some());
        }
        let dur_binary = t0.elapsed();

        // 2. Time linear scan (old implementation)
        let type_at_linear = |file: &str, line: u32, col: u32| -> Option<&str> {
            let file_id = reader.find_file_id(file)?;
            let mut best: Option<(&TypeEntry, u64)> = None;
            for entry in reader.types() {
                let span = entry.span();
                if span.file_id() == file_id && span.contains(file_id, line, col) {
                    let size = span.approximate_size();
                    match best {
                        None => best = Some((entry, size)),
                        Some((_, best_size)) if size < best_size => best = Some((entry, size)),
                        _ => {}
                    }
                }
            }
            best.map(|(e, _)| reader.get_str(e.type_offset()))
        };

        let t1 = std::time::Instant::now();
        for (file, line, col) in &samples {
            let res = type_at_linear(file, *line, *col);
            assert!(res.is_some());
        }
        let dur_linear = t1.elapsed();

        eprintln!(
            "\n[SCALE BENCH] 100,000 Types table — 100 queries:\n  Linear Scan:   {:?}\n  Binary Search: {:?}\n  Speedup:       {:.1}x",
            dur_linear,
            dur_binary,
            dur_linear.as_nanos() as f64 / dur_binary.as_nanos().max(1) as f64
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_contains_ignore_case_ascii_unit() {
        assert!(contains_ignore_case_ascii("hello world", b""));
        assert!(contains_ignore_case_ascii("hello world", b"hello"));
        assert!(contains_ignore_case_ascii("HELLO WORLD", b"hello"));
        assert!(contains_ignore_case_ascii("Hello World", b"world"));
        assert!(contains_ignore_case_ascii("Hello World", b"o w"));
        assert!(!contains_ignore_case_ascii("short", b"longer needle"));
        assert!(!contains_ignore_case_ascii("hello world", b"xyz"));
        assert!(contains_ignore_case_ascii("tokio::fs::File", b"file"));
        assert!(contains_ignore_case_ascii("tokio::fs::File", b"fs"));
        assert!(contains_ignore_case_ascii("tokio::fs::File", b"tokio"));
        // Haystack with non-ASCII UTF-8 (German/Greek)
        assert!(contains_ignore_case_ascii("Grüße World", b"world"));
        assert!(!contains_ignore_case_ascii("Grüße World", b"hello"));
        assert!(contains_ignore_case_ascii("Γειά World", b"world"));
        assert!(!contains_ignore_case_ascii("Γειά World", b"hello"));
    }

    #[test]
    fn test_search_symbols_parity_with_naive() {
        let temp_dir =
            std::env::temp_dir().join(format!("ridx_search_syms_parity_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let idx_path = temp_dir.join("test.ridx");

        let mut writer = crate::writer::IndexWriter::new("test_crate", "0.1.0");
        let raw_defs = [
            ("MyStruct", "test_crate::MyStruct"),
            ("my_function", "test_crate::my_function"),
            ("CONSTANT_VAL", "test_crate::CONSTANT_VAL"),
            ("inner_func", "test_crate::module::inner_func"),
            ("AnotherStruct", "test_crate::types::AnotherStruct"),
            ("GreekenFunktion", "test_crate::unicode::ΓειάΣας"),
        ];

        for (idx, (name, path)) in raw_defs.iter().enumerate() {
            writer.add_definition(crate::writer::RawDefinition {
                def_id: idx as u32 + 1,
                kind: crate::format::DefKind::Fn,
                visibility: 0,
                is_macro_expansion: false,
                name: name.to_string(),
                path: path.to_string(),
                file: "src/lib.rs".to_string(),
                start_line: 10,
                start_col: 1,
                end_line: 10,
                end_col: 20,
                signature: None,
            });
        }

        let mut f = std::fs::File::create(&idx_path).unwrap();
        writer.write_to(&mut f).unwrap();
        drop(f);

        let reader = IndexReader::open(&idx_path).unwrap();

        // Baseline naive matching function
        let naive_search = |q: &str| -> Vec<String> {
            let q_lower = q.to_lowercase();
            reader
                .definitions()
                .iter()
                .filter(|d| {
                    let n = reader.get_str(d.name_offset()).to_lowercase();
                    let p = reader.get_str(d.path_offset()).to_lowercase();
                    n.contains(&q_lower) || p.contains(&q_lower)
                })
                .map(|d| reader.get_str(d.name_offset()).to_string())
                .collect()
        };

        let queries = [
            "",
            "struct",
            "STRUCT",
            "my",
            "val",
            "CONSTANT",
            "module",
            "another",
            "nonexistent",
            "Γειά",
            "Σας",
        ];

        for q in queries {
            let expected = naive_search(q);
            let actual: Vec<String> = reader
                .search_symbols(q)
                .into_iter()
                .map(|d| reader.get_str(d.name_offset()).to_string())
                .collect();
            assert_eq!(
                actual, expected,
                "Query {:?} failed parity with naive search",
                q
            );
        }

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_search_symbols_scale_benchmark() {
        let temp_dir =
            std::env::temp_dir().join(format!("ridx_search_syms_scale_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let idx_path = temp_dir.join("scale.ridx");

        let mut writer = crate::writer::IndexWriter::new("scale_crate", "0.1.0");
        let n_defs = 10_000;
        for i in 0..n_defs {
            writer.add_definition(crate::writer::RawDefinition {
                def_id: i as u32 + 1,
                kind: crate::format::DefKind::Fn,
                visibility: 0,
                is_macro_expansion: false,
                name: format!("SymbolName_{}_{}", i % 50, i),
                path: format!(
                    "scale_crate::module_{}::SymbolName_{}_{}",
                    i % 20,
                    i % 50,
                    i
                ),
                file: format!("src/mod_{}.rs", i % 30),
                start_line: (i % 500 + 1) as u32,
                start_col: 1,
                end_line: (i % 500 + 1) as u32,
                end_col: 25,
                signature: None,
            });
        }

        let mut f = std::fs::File::create(&idx_path).unwrap();
        writer.write_to(&mut f).unwrap();
        drop(f);

        let reader = IndexReader::open(&idx_path).unwrap();
        let queries = [
            "symbolname",
            "module_5",
            "name_25",
            "xyz_nonexistent",
            "scale_crate",
        ];

        // 1. Time new implementation (zero-alloc ASCII)
        let t0 = std::time::Instant::now();
        for _ in 0..20 {
            for q in &queries {
                let res = reader.search_symbols(q);
                assert!(!res.is_empty() || *q == "xyz_nonexistent");
            }
        }
        let dur_new = t0.elapsed();

        // 2. Time old naive implementation
        let naive_search = |q: &str| -> Vec<&DefEntry> {
            let q_lower = q.to_lowercase();
            let mut matches = Vec::new();
            for def in reader.definitions() {
                let name = reader.get_str(def.name_offset());
                let path = reader.get_str(def.path_offset());
                if name.to_lowercase().contains(&q_lower) || path.to_lowercase().contains(&q_lower)
                {
                    matches.push(def);
                }
            }
            matches
        };

        let t1 = std::time::Instant::now();
        for _ in 0..20 {
            for q in &queries {
                let res = naive_search(q);
                assert!(!res.is_empty() || *q == "xyz_nonexistent");
            }
        }
        let dur_old = t1.elapsed();

        eprintln!(
            "\n[SCALE BENCH] 10,000 Defs — 100 queries:\n  Naive (to_lowercase): {:?}\n  Zero-Alloc ASCII:    {:?}\n  Speedup:              {:.1}x",
            dur_old,
            dur_new,
            dur_old.as_nanos() as f64 / dur_new.as_nanos().max(1) as f64
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_search_symbols_and_fallback_ignore_typaram() {
        let temp_dir =
            std::env::temp_dir().join(format!("ridx_typaram_search_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let idx_path = temp_dir.join("test.ridx");

        let mut writer = crate::writer::IndexWriter::new("serde", "1.0.0");
        writer.add_definition(crate::writer::RawDefinition {
            def_id: 1,
            kind: crate::format::DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "deserialize".to_string(),
            path: "serde::de::deserialize".to_string(),
            file: "src/de.rs".to_string(),
            start_line: 10,
            start_col: 1,
            end_line: 10,
            end_col: 20,
            signature: Some(
                "fn deserialize<'de, D>(deserializer: D) -> Result<T, D::Error>".to_string(),
            ),
        });
        writer.add_definition(crate::writer::RawDefinition {
            def_id: 2,
            kind: crate::format::DefKind::TyParam,
            visibility: 0,
            is_macro_expansion: false,
            name: "D".to_string(),
            path: "serde::de::deserialize::D".to_string(),
            file: "src/de.rs".to_string(),
            start_line: 10,
            start_col: 21,
            end_line: 10,
            end_col: 22,
            signature: None,
        });
        writer.add_definition(crate::writer::RawDefinition {
            def_id: 3,
            kind: crate::format::DefKind::TyParam,
            visibility: 0,
            is_macro_expansion: false,
            name: "T".to_string(),
            path: "serde::de::deserialize::T".to_string(),
            file: "src/de.rs".to_string(),
            start_line: 10,
            start_col: 23,
            end_line: 10,
            end_col: 24,
            signature: None,
        });
        writer.add_definition(crate::writer::RawDefinition {
            def_id: 4,
            kind: crate::format::DefKind::Struct,
            visibility: 0,
            is_macro_expansion: false,
            name: "Deserializer".to_string(),
            path: "serde::de::Deserializer".to_string(),
            file: "src/de.rs".to_string(),
            start_line: 30,
            start_col: 1,
            end_line: 30,
            end_col: 25,
            signature: None,
        });

        let mut f = std::fs::File::create(&idx_path).unwrap();
        writer.write_to(&mut f).unwrap();
        drop(f);

        let reader = IndexReader::open(&idx_path).unwrap();

        // 1. search_symbols for "deserialize" must return Fn and Struct, but NOT TyParam D or T
        let matches = reader.search_symbols("deserialize");
        let matched_names: Vec<&str> = matches
            .iter()
            .map(|d| reader.get_str(d.name_offset()))
            .collect();
        assert_eq!(matched_names, vec!["deserialize", "Deserializer"]);

        // 2. search_symbols_ascii for "d" must return "deserialize" and "Deserializer", NOT "D"
        let matches_d = reader.search_symbols_ascii(b"d");
        let names_d: Vec<&str> = matches_d
            .iter()
            .map(|d| reader.get_str(d.name_offset()))
            .collect();
        assert!(
            !names_d.contains(&"D"),
            "TyParam D must not be returned in symbol search"
        );
        assert!(names_d.contains(&"deserialize"));
        assert!(names_d.contains(&"Deserializer"));

        // 3. search_symbols for "T" must return empty, NOT TyParam T
        let matches_t = reader.search_symbols("T");
        assert!(
            matches_t.is_empty(),
            "TyParam T must not be returned in symbol search"
        );

        // 4. for_each_symbol_ascii with needle "deserialize":
        // Budget must NOT be consumed by TyParam entries
        let mut budget = 10usize;
        let mut collected = Vec::new();
        reader.for_each_symbol_ascii(b"deserialize", &mut budget, |def| {
            collected.push(reader.get_str(def.name_offset()).to_string());
            true
        });
        assert_eq!(collected, vec!["deserialize", "Deserializer"]);
        // Only 2 real items consumed budget: 10 - 2 = 8. TyParam entries did NOT consume budget!
        assert_eq!(budget, 8, "TyParam entries must not consume search budget");

        // 5. for_each_symbol_ascii_budgeted:
        // When record_budget is strictly smaller than total definitions, it stops early and returns false.
        let mut match_budget = 10usize;
        let mut record_budget = 1usize;
        let mut collected_budgeted = Vec::new();
        let completed = reader.for_each_symbol_ascii_budgeted(
            b"deserialize",
            &mut match_budget,
            &mut record_budget,
            |def| {
                collected_budgeted.push(reader.get_str(def.name_offset()).to_string());
                true
            },
        );
        assert!(
            !completed,
            "Must return false when record budget is exhausted"
        );
        assert_eq!(record_budget, 0, "Record budget must be fully exhausted");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_v3_defs_by_owner_and_derefs_roundtrip() {
        let temp_dir =
            std::env::temp_dir().join(format!("ridx_v3_roundtrip_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let idx_path = temp_dir.join("test_v3.ridx");

        let mut writer = crate::writer::IndexWriter::new("test_v3_crate", "0.1.0");

        // Add struct definition
        writer.add_definition(crate::writer::RawDefinition {
            def_id: 1,
            kind: crate::format::DefKind::Struct,
            visibility: 0,
            is_macro_expansion: false,
            name: "MyStruct".to_string(),
            path: "test_v3_crate::MyStruct".to_string(),
            file: "src/lib.rs".to_string(),
            start_line: 10,
            start_col: 1,
            end_line: 15,
            end_col: 2,
            signature: None,
        });

        // Add method with canonical owner
        writer.add_definition_with_owners(
            crate::writer::RawDefinition {
                def_id: 2,
                kind: crate::format::DefKind::AssocFn,
                visibility: 0,
                is_macro_expansion: false,
                name: "do_work".to_string(),
                path: "test_v3_crate::MyStruct::do_work".to_string(),
                file: "src/lib.rs".to_string(),
                start_line: 20,
                start_col: 5,
                end_line: 22,
                end_col: 6,
                signature: Some("pub fn do_work(&self)".to_string()),
            },
            vec!["test_v3_crate::MyStruct".to_string()],
        );

        // Add field with canonical owner
        writer.add_definition_with_owners(
            crate::writer::RawDefinition {
                def_id: 3,
                kind: crate::format::DefKind::Field,
                visibility: 0,
                is_macro_expansion: false,
                name: "count".to_string(),
                path: "test_v3_crate::MyStruct::count".to_string(),
                file: "src/lib.rs".to_string(),
                start_line: 11,
                start_col: 5,
                end_line: 11,
                end_col: 15,
                signature: None,
            },
            vec!["test_v3_crate::MyStruct".to_string()],
        );

        // Add type with canonical owner
        writer.add_type_with_owner(
            crate::writer::RawType {
                file: "src/lib.rs".to_string(),
                start_line: 30,
                start_col: 5,
                end_line: 30,
                end_col: 15,
                type_name: "MyStruct".to_string(),
            },
            Some("test_v3_crate::MyStruct".to_string()),
        );

        // Add deref mapping: String -> str
        writer.add_deref("alloc::string::String", "core::primitive::str");

        let mut f = std::fs::File::create(&idx_path).unwrap();
        writer.write_to(&mut f).unwrap();
        drop(f);

        let reader = IndexReader::open(&idx_path).unwrap();
        assert_eq!(reader.header().version, 3);
        assert!(reader.header().defs_by_owner_count >= 2);
        assert_eq!(reader.header().derefs_count, 1);

        // Verify find_definitions_by_owner
        let defs = reader.find_definitions_by_owner("test_v3_crate::MyStruct");
        assert_eq!(defs.len(), 2);
        let names: Vec<&str> = defs
            .iter()
            .map(|d| reader.get_str(d.name_offset()))
            .collect();
        assert!(names.contains(&"do_work"));
        assert!(names.contains(&"count"));

        // Verify deref lookup
        assert_eq!(
            reader.find_deref_target("alloc::string::String"),
            Some("core::primitive::str")
        );
        assert_eq!(reader.find_deref_target("core::primitive::str"), None);
        assert_eq!(
            reader.resolve_deref_chain("alloc::string::String", 3),
            vec!["core::primitive::str"]
        );

        // Verify type_entry_at
        let (ty, owner) = reader.type_entry_at("src/lib.rs", 30, 8).unwrap();
        assert_eq!(ty, "MyStruct");
        assert_eq!(owner, Some("test_v3_crate::MyStruct"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_v3_canonical_owner_collision_protection() {
        let temp_dir =
            std::env::temp_dir().join(format!("ridx_v3_collision_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let idx_path = temp_dir.join("test_collision.ridx");

        let mut writer = crate::writer::IndexWriter::new("collision_crate", "0.1.0");

        // Two types with same short name "File" in different modules
        writer.add_definition_with_owners(
            crate::writer::RawDefinition {
                def_id: 1,
                kind: crate::format::DefKind::AssocFn,
                visibility: 0,
                is_macro_expansion: false,
                name: "read_a".to_string(),
                path: "collision_crate::module_a::File::read_a".to_string(),
                file: "src/a.rs".to_string(),
                start_line: 10,
                start_col: 5,
                end_line: 12,
                end_col: 6,
                signature: None,
            },
            vec!["collision_crate::module_a::File".to_string()],
        );

        writer.add_definition_with_owners(
            crate::writer::RawDefinition {
                def_id: 2,
                kind: crate::format::DefKind::AssocFn,
                visibility: 0,
                is_macro_expansion: false,
                name: "read_b".to_string(),
                path: "collision_crate::module_b::File::read_b".to_string(),
                file: "src/b.rs".to_string(),
                start_line: 10,
                start_col: 5,
                end_line: 12,
                end_col: 6,
                signature: None,
            },
            vec!["collision_crate::module_b::File".to_string()],
        );

        let mut f = std::fs::File::create(&idx_path).unwrap();
        writer.write_to(&mut f).unwrap();
        drop(f);

        let reader = IndexReader::open(&idx_path).unwrap();

        let a_defs = reader.find_definitions_by_owner("collision_crate::module_a::File");
        assert_eq!(a_defs.len(), 1);
        assert_eq!(reader.get_str(a_defs[0].name_offset()), "read_a");

        let b_defs = reader.find_definitions_by_owner("collision_crate::module_b::File");
        assert_eq!(b_defs.len(), 1);
        assert_eq!(reader.get_str(b_defs[0].name_offset()), "read_b");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_v3_autoderef_chain_and_cycles() {
        let temp_dir = std::env::temp_dir().join(format!("ridx_v3_deref_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let idx_path = temp_dir.join("test_deref.ridx");

        let mut writer = crate::writer::IndexWriter::new("deref_crate", "0.1.0");

        // Chain: TypeA -> TypeB -> TypeC
        writer.add_deref("pkg::TypeA", "pkg::TypeB");
        writer.add_deref("pkg::TypeB", "pkg::TypeC");

        // Cycle: CycleX -> CycleY -> CycleX
        writer.add_deref("pkg::CycleX", "pkg::CycleY");
        writer.add_deref("pkg::CycleY", "pkg::CycleX");

        let mut f = std::fs::File::create(&idx_path).unwrap();
        writer.write_to(&mut f).unwrap();
        drop(f);

        let reader = IndexReader::open(&idx_path).unwrap();

        let chain = reader.resolve_deref_chain("pkg::TypeA", 5);
        assert_eq!(chain, vec!["pkg::TypeB", "pkg::TypeC"]);

        let chain_limited = reader.resolve_deref_chain("pkg::TypeA", 1);
        assert_eq!(chain_limited, vec!["pkg::TypeB"]);

        // Cycle does not loop infinitely
        let cycle_chain = reader.resolve_deref_chain("pkg::CycleX", 10);
        assert_eq!(cycle_chain, vec!["pkg::CycleY"]);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_v2_backwards_compatibility() {
        // Construct a raw v2 buffer with 128-byte HeaderV2
        let header_v2 = HeaderV2 {
            magic: MAGIC,
            version: 2,
            crate_name_offset: 0,
            crate_version_offset: 0,
            string_pool_offset: 128,
            string_pool_len: 8,
            files_offset: 136,
            files_count: 0,
            unit_key_offset: 0,
            defs_offset: 136,
            defs_count: 0,
            generation_offset: 0,
            defs_by_hash_offset: 136,
            defs_by_hash_count: 0,
            _pad2: 0,
            refs_offset: 136,
            refs_count: 0,
            _pad3: 0,
            refs_by_target_offset: 136,
            refs_by_target_count: 0,
            _pad4: 0,
            types_offset: 136,
            types_count: 0,
            _pad5: 0,
        };

        let mut buf = Vec::new();
        let header_bytes: [u8; 128] = unsafe { std::mem::transmute(header_v2.to_le()) };
        buf.extend_from_slice(&header_bytes);
        buf.extend_from_slice(b"\0crate\0\0"); // 8 bytes string pool at offset 128

        // Pad to 8-byte alignment
        while buf.len() % 8 != 0 {
            buf.push(0);
        }

        let reader = IndexReader::from_buffer(buf).unwrap();
        assert_eq!(reader.header().version, 2);
        assert_eq!(reader.header().defs_by_owner_count, 0);
        assert_eq!(reader.header().derefs_count, 0);
        assert!(reader.defs_by_owner().is_empty());
        assert!(reader.deref_entries().is_empty());
        assert_eq!(
            reader.find_definitions_by_owner("anything"),
            Vec::<&DefEntry>::new()
        );
        assert_eq!(reader.find_deref_target("anything"), None);
    }

    #[test]
    fn test_v3_owner_matching_first_interned_string() {
        let temp_dir =
            std::env::temp_dir().join(format!("ridx_first_string_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let idx_path = temp_dir.join("test_first.ridx");

        let mut writer = crate::writer::IndexWriter::new("first_crate", "0.1.0");
        // "first_crate" is the first non-empty string interned in the pool.
        // Add a type whose owner is "first_crate".
        writer.add_type_with_owner(
            crate::writer::RawType {
                file: "src/lib.rs".to_string(),
                start_line: 1,
                start_col: 1,
                end_line: 1,
                end_col: 10,
                type_name: "RootItem".to_string(),
            },
            Some("first_crate".to_string()),
        );

        let mut f = std::fs::File::create(&idx_path).unwrap();
        writer.write_to(&mut f).unwrap();
        drop(f);

        let reader = IndexReader::open(&idx_path).unwrap();
        let (ty, owner) = reader.type_entry_at("src/lib.rs", 1, 5).unwrap();
        assert_eq!(ty, "RootItem");
        // Must NOT be None (even though first_crate is the first string in pool, offset 0 is reserved for empty string)
        assert_eq!(owner, Some("first_crate"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
