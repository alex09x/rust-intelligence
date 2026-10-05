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
    pub source_root: Option<&'a Path>,
    pub unit_key: Option<&'a str>,
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
        if bytes.len() < std::mem::size_of::<Header>() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "File too small for RIDX header",
            ));
        }

        // Safely decode header into aligned memory without unaligned pointer casting
        let mut header_buf = [0u8; std::mem::size_of::<Header>()];
        header_buf.copy_from_slice(&bytes[..std::mem::size_of::<Header>()]);
        let raw_header: Header = unsafe { std::mem::transmute(header_buf) };
        let header = raw_header.from_le();

        if header.magic != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid RIDX magic: {:?}", header.magic),
            ));
        }
        if header.version != CURRENT_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unsupported RIDX version: {}", header.version),
            ));
        }

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
                        source_root: None,
                        unit_key: None,
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
                source_root: None,
                unit_key: if !target_unit.is_empty() {
                    Some(target_unit)
                } else {
                    None
                },
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
                source_root: None,
                unit_key: None,
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

    /// Retrieves the inferred expression type at a cursor position `(file, line, col)`.
    /// When nested expressions overlap, selects the tightest (innermost) span.
    pub fn type_at(&self, file: &str, line: u32, col: u32) -> Option<&str> {
        let file_id = self.find_file_id(file)?;
        let mut best: Option<(&TypeEntry, u64)> = None;

        for entry in self.types() {
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

        best.map(|(e, _)| self.get_str(e.type_offset()))
    }

    /// Fast symbol search across definitions matching a query string.
    pub fn search_symbols(&self, query: &str) -> Vec<&DefEntry> {
        let mut matches = Vec::new();
        let q = query.to_lowercase();
        for def in self.definitions() {
            let name = self.get_str(def.name_offset());
            let path = self.get_str(def.path_offset());
            if name.to_lowercase().contains(&q) || path.to_lowercase().contains(&q) {
                matches.push(def);
            }
        }
        matches
    }
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
            source_root: Some(Path::new("/workspace")),
            unit_key: Some("unit1"),
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
            source_root: None,
            unit_key: None,
        };
        assert_eq!(
            resolved.resolved_path(),
            std::path::PathBuf::from("src/lib.rs")
        );
    }
}
