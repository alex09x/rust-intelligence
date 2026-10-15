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
mod owner_and_deref_tests;
#[cfg(test)]
mod search_symbols_tests;
#[cfg(test)]
mod spans_and_sections_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod type_at_tests;
